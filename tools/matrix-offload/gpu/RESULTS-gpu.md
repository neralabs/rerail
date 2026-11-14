# GPU microprobes: results (Apple M5 Max)

## Intro

These probes ask what the Apple GPU delivers when relational work (joins, semi-joins, set intersections,
group-by counts) is restated as dense, brute-force work: bit-matrix products, int8 matrix products,
intersections over many pairs, and segmented histograms. They measure two kinds of hardware: the plain
compute threads, and the matrix units. The M5's per-core "Neural Accelerators" are reached from Metal
shaders through Metal 4 tensor operations (Metal Performance Primitives, `mpp::tensor_ops::matmul2d`). The
older 8x8 `simdgroup_matrix` path is measured too, and Apple's MPS matrix multiply is included as a
vendor-tuned reference.

Memory bandwidth is treated as a first-class result: every table gives bytes/s next to time, and G2
calibrates the ceiling.

**Machine.** Apple M5 Max (40-core GPU), 128 GB unified memory, macOS 26.4.1, Xcode 26.6 / SDK 26.5,
Swift 6.3.3. The machine was heavily loaded throughout (load average 80 to 155, mostly CPU work from other
sessions). Each table header gives the load average when that table started.

**How to read the tables.**

- *dev ms*: GPU device time of one command buffer (`gpuEndTime - gpuStartTime`), min and median over the
  repetitions (3 to 200, after one warm-up run). This is the primary measure.
- *host CPU ms*: process CPU time (user + sys, `getrusage`) spent around encode + commit + wait for one
  command buffer, median. It includes Metal's own driver threads.
- *wall ms*: wall time per command buffer, min / median. Under this load, wall time is noisy.
- Throughput is computed at the minimum device time; GB/s is given at min / median.
- Every kernel's output was checked against a CPU reference before it was timed. G3 and G4a check 256 /
  128 sampled outputs per case. G2, G5, G6 and G4b check every output (G4b every 97th row).

**Cold and warm.** Apple GPUs scale their clock with demand. A short burst after idle runs at a low
clock. "Cold" tables were taken with no pre-warm. "Warm" tables (`GPUPROBE_WARM_MS=100`) keep the GPU busy
for about 100 ms before each timed series. The difference is large (G1: about 4x per dispatch; G3 at small
sizes: up to 5x). Separately, **sustained** compute (tens of ms of back-to-back work) runs well below the
warm-burst rate on this loaded machine, while DRAM bandwidth does not drop (G2c, G4c).

Raw outputs are in the session scratchpad (`gpu-runs/*.txt`). Build and run commands are in `README.md`.

## Summary of the key numbers

| quantity | value |
|---|---|
| empty command buffer, device time | 2 µs warm, 3 to 6 µs cold |
| commit-to-completion round trip, wall | 0.13 ms warm min; 1.3 to 1.5 ms cold median under load |
| host CPU per command buffer (any size) | 0.08 to 0.35 ms |
| extra dispatch in a serial encoder, device | 0.6 µs warm (1.8 to 2.5 µs cold); concurrent encoder 0.11 µs |
| DRAM read bandwidth (256 to 512 MB) | ~550 GB/s, holding steady for 40 ms of back-to-back work |
| DRAM copy (read + write) | ~515 GB/s (kernel and blit about the same) |
| working set ≤ 32 MB (system-level cache) | 870 to 950 GB/s; 4 MB repeated: 1.15 TB/s |
| bit-matrix AND+popcount (tiled, plain ALUs) | ~70 to 76 Tbit-op/s warm burst (≈2.2 T u32 AND+popcount/s); 36 to 55 on runs ≥ 3 ms (up to 2x run-to-run) |
| MPP int8 x int8 → int32 | 106 to 110 TOPS burst (n = 2048 to 4096); ~43 TOPS sustained |
| MPP fp16 x fp16 → fp32 | 58 to 63 TOPS burst; ~32 TOPS sustained |
| MPS fp16 → fp32 (reference) | 49 to 55 TOPS burst |
| simdgroup_matrix 8x8 fp16 (simple kernel) | 12 to 14 TOPS |
| sorted u16 intersections (best mapping per L) | 330 to 470 GB/s: 6.9 G pairs/s (L=16) … 20 M pairs/s (L=4096) |
| 8 KB bitmap AND+popcount | 33 M pairs/s streaming (537 GB/s); 86 M/s from a 32 MB pool; 187 M/s from a 4 MB pool |
| segmented histogram, 30M u16 codes | 150 to 170 Gelem/s (threadgroup atomics), ~510 to 570 GB/s |
| same histogram on the matrix units (one-hot tiles) | 55 Gelem/s, correct, ~3x slower than atomics |

## G1. Dispatch floor

Empty kernel (1 threadgroup of 32 threads unless noted). "N dispatches" rows put N dispatches in one
command buffer; the device time is the total for all N, and the rate column is dispatches per second of
device time. The last two rows commit 16 or 256 command buffers back to back with completion handlers; their
columns are per command buffer (device span from the first start to the last end, divided by N).

Cold (no pre-warm):

#### G1 dispatch floor (empty kernel, 1 threadgroup x 32 threads unless noted)  (load avg 143 137 134)

| case | dev ms min | dev ms med | host CPU ms med | wall ms med | per dispatch (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| 1 dispatch / CB, waitUntilCompleted | 0.003 | 0.006 | 0.129 | 1.481 | | |
| 1 dispatch / CB, 8M threads (32768 TG x 256) | 0.204 | 0.205 | 0.100 | 0.471 | | |
| 10 dispatches, 1 serial encoder (dev ms total) | 0.028 | 0.029 | 0.092 | 0.230 | 3.529e+05 disp/s | |
| 10 dispatches, 1 concurrent encoder | 0.007 | 0.007 | 0.100 | 0.228 | 1.472e+06 disp/s | |
| 10 dispatches, 10 encoders | 0.028 | 0.029 | 0.093 | 0.218 | 3.524e+05 disp/s | |
| 100 dispatches, 1 serial encoder (dev ms total) | 0.252 | 0.255 | 0.130 | 0.928 | 3.964e+05 disp/s | |
| 100 dispatches, 1 concurrent encoder | 0.034 | 0.036 | 0.154 | 2.754 | 2.941e+06 disp/s | |
| 100 dispatches, 100 encoders | 0.252 | 0.256 | 0.184 | 2.800 | 3.972e+05 disp/s | |
| 1000 dispatches, 1 serial encoder (dev ms total) | 1.767 | 2.495 | 0.343 | 3.168 | 5.66e+05 disp/s | |
| 1000 dispatches, 1 concurrent encoder | 0.182 | 0.229 | 0.287 | 0.754 | 5.479e+06 disp/s | |
| 1000 dispatches, 1000 encoders | 1.479 | 1.806 | 0.573 | 4.729 | 6.759e+05 disp/s | |
| 1 dispatch / CB, completion handler + semaphore | 0.004 | 0.006 | 0.134 | 1.289 | | |
| 16 CBs in flight, handlers (per CB: first-start..last-end span / n) | 0.0190 | 0.0219 | 0.0505 | 0.0340 | 2.94e+04 CB/s | |
| 256 CBs in flight, handlers (per CB: first-start..last-end span / n) | 0.0178 | 0.0736 | 0.0398 | 0.0759 | 1.318e+04 CB/s | |

load avg after G1: 143 137 134


Warm (`GPUPROBE_WARM_MS=100`):

#### G1 dispatch floor (empty kernel, 1 threadgroup x 32 threads unless noted)  (load avg 121 126 130)

| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | per dispatch (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| 1 dispatch / CB, waitUntilCompleted | 0.002 | 0.002 | 0.091 | 0.136 / 0.173 | | |
| 1 dispatch / CB, 8M threads (32768 TG x 256) | 0.074 | 0.075 | 0.096 | 0.217 / 0.254 | | |
| 10 dispatches, 1 serial encoder (dev ms total) | 0.007 | 0.007 | 0.107 | 0.186 / 0.231 | 1.403e+06 disp/s | |
| 10 dispatches, 1 concurrent encoder | 0.002 | 0.002 | 0.108 | 0.172 / 0.195 | 4.8e+06 disp/s | |
| 10 dispatches, 10 encoders | 0.007 | 0.007 | 0.108 | 0.171 / 0.256 | 1.387e+06 disp/s | |
| 100 dispatches, 1 serial encoder (dev ms total) | 0.064 | 0.066 | 0.134 | 0.256 / 0.288 | 1.571e+06 disp/s | |
| 100 dispatches, 1 concurrent encoder | 0.013 | 0.013 | 0.138 | 0.194 / 0.234 | 7.767e+06 disp/s | |
| 100 dispatches, 100 encoders | 0.064 | 0.066 | 0.166 | 0.286 / 0.327 | 1.559e+06 disp/s | |
| 1000 dispatches, 1 serial encoder (dev ms total) | 0.608 | 0.630 | 0.355 | 1.014 / 1.092 | 1.644e+06 disp/s | |
| 1000 dispatches, 1 concurrent encoder | 0.111 | 0.122 | 0.343 | 0.531 / 0.640 | 9.002e+06 disp/s | |
| 1000 dispatches, 1000 encoders | 0.623 | 0.685 | 0.638 | 1.260 / 2.806 | 1.606e+06 disp/s | |
| 1 dispatch / CB, completion handler + semaphore | 0.002 | 0.002 | 0.077 | 0.126 / 0.192 | | |
| 16 CBs in flight, handlers (per CB: first-start..last-end span / n) | 0.0128 | 0.0408 | 0.0465 | 0.0504 | 1.985e+04 CB/s | |
| 256 CBs in flight, handlers (per CB: first-start..last-end span / n) | 0.0175 | 0.0307 | 0.0404 | 0.0335 | 2.986e+04 CB/s | |

load avg after G1: 121 126 130


Reading: the device-side cost of a command buffer is only microseconds. The cost is on the host side: each
command buffer round trip takes 0.13 ms at best and about 1.3 to 1.5 ms (median) when the GPU has gone
cold under this CPU load. Host CPU per command buffer is 0.1 to 0.3 ms whatever the work size. Inside one
command buffer, a serial encoder adds about 0.6 µs per dependent dispatch when warm (about 2 µs cold); a
concurrent encoder adds about 0.1 µs. Using one encoder or one encoder per dispatch costs the same. With
command buffers pipelined (in flight together), throughput is 13k to 30k command buffers per second. This
matches the repo's earlier measurement of 0.15 to 0.85 ms per wake-up.

## G2. DRAM bandwidth

Read-and-reduce (wrapping u32 sum, one partial per threadgroup, coalesced `uint4` loads with a grid stride)
and copy. All buffers use shared storage unless marked PRIVATE.

### G2a cold (first run, before the size sweep was added)

#### G2 DRAM bandwidth (shared-storage buffers unless noted)  (load avg 145 138 135)

| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | Gelem/s (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| read+reduce 256 MB, 4 uint4/thread | 0.756 | 0.788 | 0.189 | 1.848 / 7.489 | 88.71 Gu32/s | 354.9 / 340.8 |
| read+reduce 256 MB, 16 uint4/thread | 0.758 | 0.787 | 0.161 | 3.095 / 4.332 | 88.52 Gu32/s | 354.1 / 341.0 |
| read+reduce 256 MB, 64 uint4/thread | 0.780 | 0.834 | 0.167 | 2.111 / 3.723 | 86.08 Gu32/s | 344.3 / 321.8 |
| read+reduce 256 MB PRIVATE, 16 uint4/thread | 0.573 | 0.767 | 0.134 | 0.849 / 2.495 | 117.2 Gu32/s | 468.6 / 350.1 |
| read+reduce 512 MB, 4 uint4/thread | 1.257 | 1.295 | 0.163 | 2.521 / 6.054 | 106.7 Gu32/s | 427.0 / 414.5 |
| read+reduce 512 MB, 16 uint4/thread | 1.080 | 1.274 | 0.156 | 1.785 / 3.401 | 124.2 Gu32/s | 497.0 / 421.4 |
| read+reduce 512 MB, 64 uint4/thread | 1.067 | 1.322 | 0.151 | 1.618 / 3.427 | 125.8 Gu32/s | 503.2 / 406.1 |
| read+reduce 512 MB PRIVATE, 16 uint4/thread | 1.259 | 1.324 | 0.151 | 2.295 / 3.860 | 106.6 Gu32/s | 426.5 / 405.4 |
| copy kernel 256 MB -> 256 MB, 4 uint4/thread (bytes = read+write) | 1.218 | 1.261 | 0.164 | 2.372 / 10.299 | | 440.8 / 425.8 |
| copy kernel 256 MB -> 256 MB, 16 uint4/thread (bytes = read+write) | 1.242 | 1.306 | 0.156 | 2.136 / 6.562 | | 432.1 / 411.2 |
| blit copy 256 MB -> 256 MB (read+write) | 1.206 | 1.277 | 0.152 | 1.669 / 6.980 | | 445.3 / 420.3 |

load avg after G2: 143 137 134


### G2b warm, with a size sweep (cache tiers)

#### G2 DRAM bandwidth (shared-storage buffers unless noted)  (load avg 128 128 131)

| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | Gelem/s (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| read+reduce 4 MB, 16 uint4/thread | 0.006 | 0.006 | 0.111 | 0.139 / 0.203 | 163.4 Gu32/s | 653.7 / 649.4 |
| read+reduce 16 MB, 16 uint4/thread | 0.020 | 0.021 | 0.082 | 0.149 / 0.197 | 205.4 Gu32/s | 821.7 / 806.9 |
| read+reduce 32 MB, 16 uint4/thread | 0.036 | 0.037 | 0.112 | 0.195 / 0.233 | 230.6 Gu32/s | 922.5 / 900.8 |
| read+reduce 64 MB, 16 uint4/thread | 0.125 | 0.129 | 0.115 | 0.305 / 0.345 | 134.5 Gu32/s | 537.9 / 521.7 |
| read+reduce 128 MB, 16 uint4/thread | 0.257 | 0.265 | 0.135 | 0.480 / 1.241 | 130.8 Gu32/s | 523.0 / 505.6 |
| read+reduce 256 MB, 4 uint4/thread | 0.482 | 0.497 | 0.157 | 0.727 / 0.832 | 139.4 Gu32/s | 557.5 / 540.2 |
| read+reduce 256 MB, 16 uint4/thread | 0.503 | 0.509 | 0.143 | 0.725 / 0.756 | 133.4 Gu32/s | 533.5 / 527.4 |
| read+reduce 256 MB, 64 uint4/thread | 0.515 | 0.528 | 0.178 | 0.757 / 0.879 | 130.3 Gu32/s | 521.0 / 508.2 |
| read+reduce 256 MB PRIVATE, 16 uint4/thread | 0.486 | 0.499 | 0.137 | 0.727 / 1.041 | 138.2 Gu32/s | 552.6 / 537.9 |
| read+reduce 512 MB, 4 uint4/thread | 0.961 | 1.002 | 0.202 | 1.201 / 1.946 | 139.6 Gu32/s | 558.5 / 535.6 |
| read+reduce 512 MB, 16 uint4/thread | 0.979 | 0.990 | 0.205 | 1.270 / 2.143 | 137.2 Gu32/s | 548.7 / 542.1 |
| read+reduce 512 MB, 64 uint4/thread | 0.986 | 0.992 | 0.223 | 1.448 / 2.419 | 136.1 Gu32/s | 544.6 / 541.0 |
| read+reduce 512 MB PRIVATE, 16 uint4/thread | 0.951 | 0.960 | 0.222 | 1.256 / 1.395 | 141.2 Gu32/s | 564.8 / 559.4 |
| copy kernel 256 MB -> 256 MB, 4 uint4/thread (bytes = read+write) | 1.075 | 1.099 | 0.214 | 1.737 / 4.194 | | 499.5 / 488.4 |
| copy kernel 256 MB -> 256 MB, 16 uint4/thread (bytes = read+write) | 1.044 | 1.068 | 0.144 | 1.291 / 1.879 | | 514.4 / 502.6 |
| blit copy 256 MB -> 256 MB (read+write) | 1.034 | 1.206 | 0.215 | 1.353 / 4.506 | | 519.3 / 445.3 |

load avg after G2: 131 128 131


### G2c sustained: each reduce encoded 40 times per command buffer (warm)

#### G2 DRAM bandwidth (shared-storage buffers unless noted)  (load avg 94 114 125)

| read+reduce 4 MB, 16 uint4/thread x40 per CB | 0.145 | 0.146 | 0.131 | 0.313 / 0.370 | 288.3 Gu32/s | 1153.1 / 1147.5 |
| read+reduce 16 MB, 16 uint4/thread x40 per CB | 0.769 | 0.772 | 0.135 | 0.972 / 1.012 | 218.1 Gu32/s | 872.3 / 869.1 |
| read+reduce 32 MB, 16 uint4/thread x40 per CB | 1.411 | 1.412 | 0.180 | 1.648 / 1.683 | 237.8 Gu32/s | 951.3 / 950.3 |
| read+reduce 64 MB, 16 uint4/thread x40 per CB | 4.631 | 4.811 | 0.223 | 5.058 / 5.634 | 144.9 Gu32/s | 579.7 / 558.0 |
| read+reduce 128 MB, 16 uint4/thread x40 per CB | 10.183 | 10.279 | 0.242 | 10.530 / 10.754 | 131.8 Gu32/s | 527.2 / 522.3 |
| read+reduce 256 MB, 4 uint4/thread x40 per CB | 19.507 | 19.894 | 0.256 | 19.841 / 20.620 | 137.6 Gu32/s | 550.4 / 539.7 |
| read+reduce 256 MB, 16 uint4/thread x40 per CB | 19.386 | 19.858 | 0.256 | 19.779 / 20.248 | 138.5 Gu32/s | 553.9 / 540.7 |
| read+reduce 256 MB, 64 uint4/thread x40 per CB | 19.472 | 19.865 | 0.338 | 20.255 / 21.598 | 137.9 Gu32/s | 551.4 / 540.5 |
| read+reduce 512 MB, 4 uint4/thread x40 per CB | 39.062 | 39.638 | 0.263 | 39.773 / 40.319 | 137.4 Gu32/s | 549.8 / 541.8 |
| read+reduce 512 MB, 16 uint4/thread x40 per CB | 38.825 | 39.157 | 0.249 | 39.279 / 39.739 | 138.3 Gu32/s | 553.1 / 548.4 |
| read+reduce 512 MB, 64 uint4/thread x40 per CB | 38.066 | 39.184 | 0.356 | 39.002 / 39.827 | 141 Gu32/s | 564.1 / 548.1 |

load avg after G2: 107 116 126


Reading: the DRAM ceiling is about **550 GB/s read** and **~515 GB/s copy**. It holds through 40 ms of
continuous streaming. Private storage is no faster than shared storage (within noise). A blit copy runs at
the same speed as a kernel copy. Working sets of 32 MB or less are served from the system-level cache at
870 to 950 GB/s (1.15 TB/s for 4 MB repeated). At 64 MB the rate drops to DRAM speed, so the cache is
between 32 and 64 MB. Cold single bursts reach only 350 to 500 GB/s, because the fixed ramp-up cost is
significant against a 0.5 to 1 ms kernel.

## G3. Bit-matrix AND + popcount product

C[i][j] = popcount(A_i & B_j). A and B are n x k bits, stored as rows of u32 words (50% random bits), and C
is n x n u32. "Gbit-op/s" is n·n·k / time, i.e. one bit-AND-and-count per bit; divide by 32 for u32
AND+popcount operations. Tiled kernels: 256 threads per threadgroup; 16·TM rows of A and of B are staged
KC words at a time in threadgroup memory (8 KB); each thread owns TM x TM outputs. The naive kernel is
skipped above 1.2·10^12 bit-ops.

Cold:

#### G3 bit-matrix C[i][j] = popcount(A_i & B_j), n = m; output u32  (load avg 153 140 135)

| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | Gbit-op/s (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| n=m=1024, k=1024 bits: naive (1 thread/output) | 0.110 | 0.110 | 0.115 | 0.599 / 0.696 | 9754 Gbit-op/s | 40.5 / 40.3 |
| n=m=1024, k=1024 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 0.084 | 0.085 | 0.136 | 0.680 / 2.744 | 1.271e+04 Gbit-op/s | 52.7 / 52.6 |
| n=m=1024, k=1024 bits: tiled 128x128, 8x8/thread, KC=8 (8 KB tg) | 0.187 | 0.189 | 0.133 | 0.427 / 2.661 | 5742 Gbit-op/s | 23.8 / 23.5 |
| n=m=1024, k=16384 bits: naive (1 thread/output) | 1.418 | 1.420 | 0.116 | 1.625 / 1.852 | 1.211e+04 Gbit-op/s | 5.9 / 5.9 |
| n=m=1024, k=16384 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 1.134 | 1.145 | 0.147 | 1.396 / 5.556 | 1.516e+04 Gbit-op/s | 7.4 / 7.3 |
| n=m=1024, k=16384 bits: tiled 128x128, 8x8/thread, KC=8 (8 KB tg) | 2.684 | 2.692 | 0.169 | 4.605 / 7.386 | 6401 Gbit-op/s | 3.1 / 3.1 |
| n=m=1024, k=65536 bits: naive (1 thread/output) | 3.757 | 5.727 | 0.174 | 3.963 / 7.371 | 1.829e+04 Gbit-op/s | 5.6 / 3.7 |
| n=m=1024, k=65536 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 1.109 | 1.228 | 0.152 | 2.415 / 3.330 | 6.197e+04 Gbit-op/s | 18.9 / 17.1 |
| n=m=1024, k=65536 bits: tiled 128x128, 8x8/thread, KC=8 (8 KB tg) | 2.406 | 2.581 | 0.151 | 3.280 / 4.234 | 2.856e+04 Gbit-op/s | 8.7 / 8.1 |
| n=m=2048, k=4096 bits: naive (1 thread/output) | 0.298 | 0.392 | 0.135 | 0.978 / 3.681 | 5.772e+04 Gbit-op/s | 63.4 / 48.1 |
| n=m=2048, k=4096 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 0.206 | 0.346 | 0.151 | 1.006 / 4.021 | 8.36e+04 Gbit-op/s | 91.8 / 54.5 |
| n=m=2048, k=4096 bits: tiled 128x128, 8x8/thread, KC=8 (8 KB tg) | 0.579 | 0.716 | 0.177 | 1.588 / 5.006 | 2.969e+04 Gbit-op/s | 32.6 / 26.3 |
| n=m=4096, k=4096 bits: naive (1 thread/output) | 1.357 | 1.561 | 0.172 | 2.714 / 7.002 | 5.064e+04 Gbit-op/s | 52.5 / 45.7 |
| n=m=4096, k=4096 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 1.092 | 1.168 | 0.142 | 1.686 / 3.840 | 6.29e+04 Gbit-op/s | 65.3 / 61.0 |
| n=m=4096, k=4096 bits: tiled 128x128, 8x8/thread, KC=8 (8 KB tg) | 3.153 | 3.635 | 0.166 | 4.053 / 4.904 | 2.18e+04 Gbit-op/s | 22.6 / 19.6 |
| n=m=4096, k=16384 bits: naive (1 thread/output) | 4.743 | 4.976 | 0.180 | 6.530 / 9.413 | 5.795e+04 Gbit-op/s | 17.7 / 16.9 |
| n=m=4096, k=16384 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 3.393 | 3.574 | 0.181 | 6.633 / 10.666 | 8.102e+04 Gbit-op/s | 24.7 / 23.5 |
| n=m=4096, k=16384 bits: tiled 128x128, 8x8/thread, KC=8 (8 KB tg) | 8.392 | 8.775 | 0.192 | 11.372 / 13.101 | 3.276e+04 Gbit-op/s | 10.0 / 9.6 |
| n=m=8192, k=2048 bits: naive (1 thread/output) | 2.752 | 2.918 | 0.178 | 3.591 / 5.222 | 4.994e+04 Gbit-op/s | 99.1 / 93.4 |
| n=m=8192, k=2048 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 1.885 | 2.074 | 0.200 | 4.046 / 6.265 | 7.29e+04 Gbit-op/s | 144.6 / 131.4 |
| n=m=8192, k=2048 bits: tiled 128x128, 8x8/thread, KC=8 (8 KB tg) | 4.647 | 4.832 | 0.179 | 5.116 / 5.238 | 2.958e+04 Gbit-op/s | 58.7 / 56.4 |
| n=m=8192, k=8192 bits: naive (1 thread/output) | 12.339 | 13.269 | 0.207 | 13.265 / 16.407 | 4.455e+04 Gbit-op/s | 23.1 / 21.5 |
| n=m=8192, k=8192 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 9.923 | 10.558 | 0.223 | 12.393 / 14.888 | 5.54e+04 Gbit-op/s | 28.7 / 27.0 |
| n=m=8192, k=8192 bits: tiled 128x128, 8x8/thread, KC=8 (8 KB tg) | 30.796 | 34.920 | 0.221 | 33.140 / 40.093 | 1.785e+04 Gbit-op/s | 9.3 / 8.2 |

(bytes = unique operand + output bytes, A + B + C, once; 256 random (i,j) outputs checked per case)

load avg after G3: 153 140 135


Warm, run 1:

#### G3 bit-matrix C[i][j] = popcount(A_i & B_j), n = m; output u32  (load avg 153 142 136)

| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | Gbit-op/s (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| n=m=1024, k=1024 bits: naive (1 thread/output) | 0.023 | 0.023 | 0.097 | 0.197 / 0.227 | 4.685e+04 Gbit-op/s | 194.5 / 193.8 |
| n=m=1024, k=1024 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 0.018 | 0.019 | 0.166 | 0.668 / 3.817 | 6.035e+04 Gbit-op/s | 250.5 / 235.1 |
| n=m=1024, k=1024 bits: tiled 128x128, 8x8/thread, KC=8 (8 KB tg) | 0.040 | 0.040 | 0.112 | 0.218 / 0.229 | 2.71e+04 Gbit-op/s | 112.5 / 112.3 |
| n=m=1024, k=16384 bits: naive (1 thread/output) | 0.297 | 0.322 | 0.171 | 2.732 / 4.515 | 5.789e+04 Gbit-op/s | 28.3 / 26.0 |
| n=m=1024, k=16384 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 0.227 | 0.238 | 0.125 | 0.413 / 0.433 | 7.565e+04 Gbit-op/s | 36.9 / 35.3 |
| n=m=1024, k=16384 bits: tiled 128x128, 8x8/thread, KC=8 (8 KB tg) | 0.560 | 0.562 | 0.193 | 2.551 / 5.465 | 3.068e+04 Gbit-op/s | 15.0 / 14.9 |
| n=m=1024, k=65536 bits: naive (1 thread/output) | 1.199 | 1.207 | 0.155 | 1.448 / 1.466 | 5.73e+04 Gbit-op/s | 17.5 / 17.4 |
| n=m=1024, k=65536 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 0.966 | 0.968 | 0.180 | 1.232 / 1.280 | 7.112e+04 Gbit-op/s | 21.7 / 21.7 |
| n=m=1024, k=65536 bits: tiled 128x128, 8x8/thread, KC=8 (8 KB tg) | 2.487 | 2.549 | 0.209 | 2.886 / 6.936 | 2.763e+04 Gbit-op/s | 8.4 / 8.2 |
| n=m=2048, k=4096 bits: naive (1 thread/output) | 0.328 | 0.328 | 0.147 | 0.526 / 0.590 | 5.244e+04 Gbit-op/s | 57.6 / 57.6 |
| n=m=2048, k=4096 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 0.226 | 0.231 | 0.261 | 0.438 / 13.167 | 7.596e+04 Gbit-op/s | 83.5 / 81.9 |
| n=m=2048, k=4096 bits: tiled 128x128, 8x8/thread, KC=8 (8 KB tg) | 0.631 | 0.655 | 0.148 | 0.860 / 0.904 | 2.721e+04 Gbit-op/s | 29.9 / 28.8 |
| n=m=4096, k=4096 bits: naive (1 thread/output) | 1.389 | 1.396 | 0.223 | 1.681 / 2.384 | 4.948e+04 Gbit-op/s | 51.3 / 51.1 |
| n=m=4096, k=4096 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 0.939 | 1.010 | 0.225 | 1.246 / 1.359 | 7.317e+04 Gbit-op/s | 75.9 / 70.6 |
| n=m=4096, k=4096 bits: tiled 128x128, 8x8/thread, KC=8 (8 KB tg) | 2.590 | 2.640 | 0.407 | 3.563 / 3.894 | 2.653e+04 Gbit-op/s | 27.5 / 27.0 |
| n=m=4096, k=16384 bits: naive (1 thread/output) | 6.924 | 7.470 | 0.517 | 7.839 / 15.120 | 3.97e+04 Gbit-op/s | 12.1 / 11.2 |
| n=m=4096, k=16384 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 5.691 | 6.130 | 0.488 | 7.071 / 20.680 | 4.83e+04 Gbit-op/s | 14.7 / 13.7 |
| n=m=4096, k=16384 bits: tiled 128x128, 8x8/thread, KC=8 (8 KB tg) | 17.683 | 19.347 | 0.458 | 20.521 / 22.263 | 1.554e+04 Gbit-op/s | 4.7 / 4.3 |
| n=m=8192, k=2048 bits: naive (1 thread/output) | 4.059 | 4.402 | 0.391 | 5.351 / 18.630 | 3.386e+04 Gbit-op/s | 67.2 / 61.9 |
| n=m=8192, k=2048 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 2.768 | 2.913 | 0.271 | 3.208 / 4.127 | 4.965e+04 Gbit-op/s | 98.5 / 93.6 |
| n=m=8192, k=2048 bits: tiled 128x128, 8x8/thread, KC=8 (8 KB tg) | 6.359 | 6.679 | 0.223 | 6.723 / 7.051 | 2.161e+04 Gbit-op/s | 42.9 / 40.8 |
| n=m=8192, k=8192 bits: naive (1 thread/output) | 16.699 | 18.719 | 0.253 | 17.065 / 19.115 | 3.292e+04 Gbit-op/s | 17.1 / 15.2 |
| n=m=8192, k=8192 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 14.696 | 15.486 | 0.366 | 17.691 / 18.423 | 3.741e+04 Gbit-op/s | 19.4 / 18.4 |
| n=m=8192, k=8192 bits: tiled 128x128, 8x8/thread, KC=8 (8 KB tg) | 38.839 | 43.735 | 0.265 | 41.874 / 44.081 | 1.415e+04 Gbit-op/s | 7.3 / 6.5 |

(bytes = unique operand + output bytes, A + B + C, once; 256 random (i,j) outputs checked per case)

load avg after G3: 153 142 136


Warm, run 2 (tiled 64x64 only):

#### G3 bit-matrix C[i][j] = popcount(A_i & B_j), n = m; output u32  (load avg 121 126 130)

| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | Gbit-op/s (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| n=m=1024, k=1024 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 0.018 | 0.018 | 0.121 | 0.247 / 0.289 | 6.092e+04 Gbit-op/s | 252.8 / 251.1 |
| n=m=1024, k=16384 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 0.246 | 0.249 | 0.198 | 1.066 / 3.159 | 6.997e+04 Gbit-op/s | 34.2 / 33.7 |
| n=m=1024, k=65536 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 0.977 | 1.046 | 0.270 | 2.105 / 4.020 | 7.03e+04 Gbit-op/s | 21.5 / 20.0 |
| n=m=2048, k=4096 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 0.242 | 0.242 | 0.182 | 0.506 / 0.548 | 7.098e+04 Gbit-op/s | 78.0 / 78.0 |
| n=m=4096, k=4096 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 1.010 | 1.010 | 0.301 | 1.581 / 1.752 | 6.804e+04 Gbit-op/s | 70.6 / 70.6 |
| n=m=4096, k=16384 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 5.780 | 6.015 | 0.360 | 6.459 / 7.638 | 4.755e+04 Gbit-op/s | 14.5 / 13.9 |
| n=m=8192, k=2048 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 3.782 | 4.025 | 0.336 | 4.339 / 4.867 | 3.634e+04 Gbit-op/s | 72.1 / 67.7 |
| n=m=8192, k=8192 bits: tiled 64x64, 4x4/thread, KC=16 (8 KB tg) | 13.286 | 14.658 | 0.319 | 14.701 / 16.026 | 4.138e+04 Gbit-op/s | 21.5 / 19.5 |

(bytes = unique operand + output bytes, A + B + C, once; 256 random (i,j) outputs checked per case)

load avg after G3: 115 125 130


Reading: the 64x64 tile with 4x4 outputs per thread is the best of the three, at about **70 to 76
Tbit-op/s** in warm bursts (≈2.2 to 2.4·10^12 u32 AND+popcount per second). That is close to what the
ALUs can issue (40 cores x 128 lanes x ~1.7 GHz ≈ 8.7·10^12 simple ops/s; each word costs an AND, a
popcount and an add). Longer runs (≥ 3 ms) drop to 36 to 55 Tbit-op/s, and the same case differs by up
to 2x between runs (the cold run was sometimes the fastest) (sustained-clock limit, see G4c). The 128x128 / 8x8 variant is 2.5x slower
(register pressure). The naive kernel is only 20 to 30% behind the tiled one because the caches serve its
re-reads. This kernel is compute-bound: unique bytes/s are far below DRAM speed except at k = 1024.

## G4. Matrix units

### What the SDK exposes, and what failed

- **Metal 4 tensors and MPP are reachable from run-time compiled MSL.** `#include <metal_tensor>` and
  `#include <MetalPerformancePrimitives/MetalPerformancePrimitives.h>` compile with
  `MTLLanguageVersion.version4_0` (headers live in `/System/Library/Frameworks/MetalPerformancePrimitives.framework`
  and the GPUCompiler private framework). No offline toolchain is needed. `supportsFamily(.metal4)` and
  `.apple10` are both true.
- The tensors are **inline tensors over plain device buffers**:
  `tensor<device T, dextents<int32_t,2>, tensor_inline>(ptr, dextents(cols, rows))`. No `MTLTensor` objects
  or Metal 4 argument tables are needed.
- **The header's own examples do not compile.** `static_slice<...>` does not exist; the SDK spells it
  `slice<E0, E1>(x, y)`. The cooperative tensor's `get_mask(i)` does not exist; the SDK's method is
  `is_valid_element(i)`. `#pragma unroll full` is rejected. With those three fixed, everything below
  compiles and verifies.
- Verified type combinations: fp16 x fp16 → fp32, fp16 x fp16 → fp16, int8 x int8 → int32, from device
  memory. Also int8 one-hot tiles from **threadgroup** memory accumulated into a cooperative tensor (G6).
- MPSGraph was not needed as a fallback and was not measured. MPS `MPSMatrixMultiplication` (fp16 → fp32)
  is included as Apple's own reference.

### G4a square matmul, warm

Inputs are integers in [-2, 2], so fp32 and int32 accumulation is exact (fp16 output is checked to 0.2%).
TOPS = 2·M·N·K / time. Bytes = A + B + C once.

#### G4a Metal 4 tensor ops (MPP matmul2d) square M=N=K; C = A*B  (load avg 97 127 131)

| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | TOPS (2MNK) (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| n=1024 MPP fp16 x fp16 -> fp32, 64x32 tile, 4 SG | 0.071 | 0.071 | 0.101 | 0.252 / 0.263 | 30.34 TOPS | 118.5 / 118.4 |
| n=1024 MPP fp16 x fp16 -> fp32, 128x64 tile, 4 SG | 0.046 | 0.046 | 0.110 | 0.219 / 0.245 | 47.15 TOPS | 184.2 / 183.4 |
| n=1024 MPP fp16 x fp16 -> fp32, 128x128 tile, 4 SG | 0.056 | 0.056 | 0.109 | 0.229 / 0.256 | 38.21 TOPS | 149.2 / 148.8 |
| n=1024 MPP fp16 x fp16 -> fp16, 64x32 tile, 4 SG | 0.071 | 0.071 | 0.110 | 0.254 / 0.267 | 30.23 TOPS | 88.6 / 88.3 |
| n=1024 MPP int8 x int8 -> int32, 64x32 tile, 4 SG | 0.040 | 0.040 | 0.098 | 0.217 / 0.238 | 54.2 TOPS | 158.8 / 158.3 |
| n=1024 MPP int8 x int8 -> int32, 128x64 tile, 4 SG | 0.026 | 0.026 | 0.118 | 0.209 / 0.254 | 84.08 TOPS | 246.3 / 244.7 |
| n=1024 MPP int8 x int8 -> int32, 128x128 tile, 4 SG | 0.043 | 0.043 | 0.113 | 0.228 / 0.239 | 50.04 TOPS | 146.6 / 145.7 |
| n=1024 MPP int8 x int8 -> int32, 256x128 tile, 8 SG | 0.042 | 0.042 | 0.124 | 0.224 / 0.236 | 51.08 TOPS | 149.6 / 148.3 |
| n=1024 simdgroup_matrix 8x8 fp16 -> fp32, 64x64/TG | 0.174 | 0.175 | 0.131 | 0.373 / 0.396 | 12.36 TOPS | 48.3 / 48.0 |
| n=1024 MPSMatrixMultiplication fp16 -> fp32 | 0.039 | 0.040 | 0.230 | 0.437 / 1.525 | 54.6 TOPS | 213.3 / 211.0 |
| n=2048 MPP fp16 x fp16 -> fp32, 64x32 tile, 4 SG | 0.537 | 0.538 | 0.137 | 0.739 / 0.781 | 31.98 TOPS | 62.5 / 62.4 |
| n=2048 MPP fp16 x fp16 -> fp32, 128x64 tile, 4 SG | 0.293 | 0.295 | 0.148 | 0.494 / 0.525 | 58.56 TOPS | 114.4 / 113.9 |
| n=2048 MPP fp16 x fp16 -> fp32, 128x128 tile, 4 SG | 0.319 | 0.326 | 0.170 | 0.549 / 0.610 | 53.81 TOPS | 105.1 / 103.0 |
| n=2048 MPP fp16 x fp16 -> fp16, 64x32 tile, 4 SG | 0.552 | 0.556 | 0.167 | 0.808 / 0.865 | 31.15 TOPS | 45.6 / 45.3 |
| n=2048 MPP int8 x int8 -> int32, 64x32 tile, 4 SG | 0.324 | 0.327 | 0.165 | 0.507 / 0.582 | 52.98 TOPS | 77.6 / 77.0 |
| n=2048 MPP int8 x int8 -> int32, 128x64 tile, 4 SG | 0.162 | 0.165 | 0.151 | 0.373 / 0.432 | 106.2 TOPS | 155.5 / 152.2 |
| n=2048 MPP int8 x int8 -> int32, 128x128 tile, 4 SG | 0.210 | 0.211 | 0.200 | 0.451 / 0.603 | 81.81 TOPS | 119.8 / 119.4 |
| n=2048 MPP int8 x int8 -> int32, 256x128 tile, 8 SG | 0.253 | 0.265 | 0.143 | 0.465 / 0.494 | 67.98 TOPS | 99.6 / 94.8 |
| n=2048 simdgroup_matrix 8x8 fp16 -> fp32, 64x64/TG | 1.396 | 1.471 | 0.307 | 2.757 / 2.824 | 12.3 TOPS | 24.0 / 22.8 |
| n=2048 MPSMatrixMultiplication fp16 -> fp32 | 0.353 | 0.380 | 0.478 | 1.635 / 1.719 | 48.62 TOPS | 95.0 / 88.2 |
| n=4096 MPP fp16 x fp16 -> fp32, 64x32 tile, 4 SG | 4.498 | 4.789 | 0.246 | 5.995 / 6.700 | 30.56 TOPS | 29.8 / 28.0 |
| n=4096 MPP fp16 x fp16 -> fp32, 128x64 tile, 4 SG | 2.470 | 2.580 | 0.231 | 2.821 / 4.682 | 55.64 TOPS | 54.3 / 52.0 |
| n=4096 MPP fp16 x fp16 -> fp32, 128x128 tile, 4 SG | 2.966 | 3.188 | 0.187 | 3.334 / 3.713 | 46.33 TOPS | 45.2 / 42.1 |
| n=4096 MPP fp16 x fp16 -> fp16, 64x32 tile, 4 SG | 4.988 | 5.197 | 0.183 | 5.249 / 5.538 | 27.55 TOPS | 20.2 / 19.4 |
| n=4096 MPP int8 x int8 -> int32, 64x32 tile, 4 SG | 3.072 | 3.131 | 0.204 | 3.413 / 3.447 | 44.73 TOPS | 32.8 / 32.2 |
| n=4096 MPP int8 x int8 -> int32, 128x64 tile, 4 SG | 1.521 | 1.538 | 0.185 | 1.826 / 1.838 | 90.35 TOPS | 66.2 / 65.4 |
| n=4096 MPP int8 x int8 -> int32, 128x128 tile, 4 SG | 2.224 | 2.402 | 0.183 | 2.513 / 2.741 | 61.8 TOPS | 45.3 / 41.9 |
| n=4096 MPP int8 x int8 -> int32, 256x128 tile, 8 SG | 2.715 | 2.930 | 0.208 | 3.006 / 3.250 | 50.62 TOPS | 37.1 / 34.4 |
| n=4096 simdgroup_matrix 8x8 fp16 -> fp32, 64x64/TG | 14.181 | 15.053 | 0.277 | 14.648 / 16.117 | 9.692 TOPS | 9.5 / 8.9 |
| n=4096 MPSMatrixMultiplication fp16 -> fp32 | 4.079 | 4.403 | 0.457 | 4.647 / 4.940 | 33.69 TOPS | 32.9 / 30.5 |
| n=8192 MPP fp16 x fp16 -> fp32, 64x32 tile, 4 SG | 54.666 | 59.818 | 0.232 | 59.256 / 60.271 | 20.11 TOPS | 9.8 / 9.0 |
| n=8192 MPP fp16 x fp16 -> fp32, 128x64 tile, 4 SG | 83.293 | 94.514 | 0.311 | 84.217 / 95.010 | 13.2 TOPS | 6.4 / 5.7 |
| n=8192 MPP fp16 x fp16 -> fp32, 128x128 tile, 4 SG | 90.725 | 114.994 | 0.309 | 91.043 / 118.930 | 12.12 TOPS | 5.9 / 4.7 |
| n=8192 MPP fp16 x fp16 -> fp16, 64x32 tile, 4 SG | 126.898 | 137.281 | 0.252 | 128.211 / 137.595 | 8.665 TOPS | 3.2 / 2.9 |
| n=8192 MPP int8 x int8 -> int32, 64x32 tile, 4 SG | 62.929 | 70.350 | 0.223 | 63.505 / 71.894 | 17.47 TOPS | 6.4 / 5.7 |
| n=8192 MPP int8 x int8 -> int32, 128x64 tile, 4 SG | 32.092 | 34.135 | 0.256 | 33.113 / 34.530 | 34.26 TOPS | 12.5 / 11.8 |
| n=8192 MPP int8 x int8 -> int32, 128x128 tile, 4 SG | 40.547 | 43.850 | 0.235 | 41.751 / 44.423 | 27.12 TOPS | 9.9 / 9.2 |
| n=8192 MPP int8 x int8 -> int32, 256x128 tile, 8 SG | 36.552 | 40.204 | 0.304 | 37.122 / 41.739 | 30.08 TOPS | 11.0 / 10.0 |
| n=8192 simdgroup_matrix 8x8 fp16 -> fp32, 64x64/TG | 177.708 | 183.766 | 0.265 | 178.211 / 184.229 | 6.187 TOPS | 3.0 / 2.9 |
| n=8192 MPSMatrixMultiplication fp16 -> fp32 | 40.589 | 42.646 | 0.356 | 41.050 / 43.160 | 27.09 TOPS | 13.2 / 12.6 |

(bytes = A + B + C once; inputs are integers in [-2, 2], 128 sampled outputs checked exactly (fp16 output: 0.2% tolerance))


Swizzled threadgroup order (blocked raster over 8 or 16 tile-rows), separate warm run:

#### G4a Metal 4 tensor ops (MPP matmul2d) square M=N=K; C = A*B  (load avg 126 130 132)

| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | TOPS (2MNK) (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| n=4096 MPP fp16 x fp16 -> fp32, 64x32 tile, 4 SG | 4.269 | 4.271 | 0.186 | 4.557 / 4.568 | 32.19 TOPS | 31.4 / 31.4 |
| n=4096 MPP fp16 x fp16 -> fp32, 128x64 tile, 4 SG | 2.175 | 2.186 | 0.193 | 2.453 / 2.514 | 63.2 TOPS | 61.7 / 61.4 |
| n=4096 MPP fp16 x fp16 -> fp32, 128x128 tile, 4 SG | 2.434 | 2.581 | 0.202 | 2.744 / 2.840 | 56.47 TOPS | 55.2 / 52.0 |
| n=4096 MPP fp16 x fp16 -> fp16, 64x32 tile, 4 SG | 4.457 | 4.554 | 0.198 | 4.854 / 4.903 | 30.83 TOPS | 22.6 / 22.1 |
| n=4096 MPP int8 x int8 -> int32, 64x32 tile, 4 SG | 2.691 | 2.752 | 0.225 | 3.087 / 3.558 | 51.08 TOPS | 37.4 / 36.6 |
| n=4096 MPP int8 x int8 -> int32, 128x64 tile, 4 SG | 1.250 | 1.259 | 0.180 | 1.523 / 1.558 | 110 TOPS | 80.6 / 80.0 |
| n=4096 MPP fp16 x fp16 -> fp32, 128x64 tile, 4 SG, swizzle 8 | 2.436 | 2.643 | 0.209 | 2.941 / 3.079 | 56.42 TOPS | 55.1 / 50.8 |
| n=4096 MPP int8 x int8 -> int32, 128x64 tile, 4 SG, swizzle 8 | 1.333 | 1.378 | 0.196 | 1.699 / 1.713 | 103.1 TOPS | 75.5 / 73.1 |
| n=4096 MPP int8 x int8 -> int32, 128x64 tile, 4 SG, swizzle 16 | 1.422 | 1.440 | 0.155 | 1.710 / 1.733 | 96.65 TOPS | 70.8 / 69.9 |
| n=4096 MPP int8 x int8 -> int32, 128x128 tile, 4 SG | 2.258 | 2.318 | 0.177 | 2.517 / 2.709 | 60.86 TOPS | 44.6 / 43.4 |
| n=4096 MPP int8 x int8 -> int32, 256x128 tile, 8 SG | 2.554 | 3.061 | 0.217 | 3.006 / 3.533 | 53.81 TOPS | 39.4 / 32.9 |
| n=4096 simdgroup_matrix 8x8 fp16 -> fp32, 64x64/TG | 14.260 | 15.611 | 0.291 | 15.469 / 17.046 | 9.638 TOPS | 9.4 / 8.6 |
| n=4096 MPSMatrixMultiplication fp16 -> fp32 | 4.377 | 4.725 | 0.631 | 5.136 / 5.811 | 31.4 TOPS | 30.7 / 28.4 |
| n=8192 MPP fp16 x fp16 -> fp32, 64x32 tile, 4 SG | 57.826 | 58.372 | 0.223 | 58.161 / 59.062 | 19.01 TOPS | 9.3 / 9.2 |
| n=8192 MPP fp16 x fp16 -> fp32, 128x64 tile, 4 SG | 77.336 | 81.762 | 0.286 | 78.209 / 82.212 | 14.22 TOPS | 6.9 / 6.6 |
| n=8192 MPP fp16 x fp16 -> fp32, 128x128 tile, 4 SG | 111.600 | 119.604 | 0.283 | 112.051 / 119.951 | 9.852 TOPS | 4.8 / 4.5 |
| n=8192 MPP fp16 x fp16 -> fp16, 64x32 tile, 4 SG | 129.534 | 135.718 | 0.311 | 130.114 / 136.166 | 8.488 TOPS | 3.1 / 3.0 |
| n=8192 MPP int8 x int8 -> int32, 64x32 tile, 4 SG | 60.635 | 72.609 | 0.274 | 61.039 / 75.646 | 18.13 TOPS | 6.6 / 5.5 |
| n=8192 MPP int8 x int8 -> int32, 128x64 tile, 4 SG | 28.766 | 30.538 | 0.245 | 29.125 / 31.124 | 38.22 TOPS | 14.0 / 13.2 |
| n=8192 MPP fp16 x fp16 -> fp32, 128x64 tile, 4 SG, swizzle 8 | 70.438 | 74.583 | 0.226 | 70.811 / 75.360 | 15.61 TOPS | 7.6 / 7.2 |
| n=8192 MPP int8 x int8 -> int32, 128x64 tile, 4 SG, swizzle 8 | 21.395 | 22.233 | 0.275 | 21.828 / 24.880 | 51.39 TOPS | 18.8 / 18.1 |
| n=8192 MPP int8 x int8 -> int32, 128x64 tile, 4 SG, swizzle 16 | 25.875 | 27.264 | 0.275 | 26.421 / 29.130 | 42.49 TOPS | 15.6 / 14.8 |
| n=8192 MPP int8 x int8 -> int32, 128x128 tile, 4 SG | 47.737 | 55.147 | 0.243 | 48.103 / 55.662 | 23.03 TOPS | 8.4 / 7.3 |
| n=8192 MPP int8 x int8 -> int32, 256x128 tile, 8 SG | 34.362 | 38.709 | 0.244 | 34.797 / 39.564 | 32 TOPS | 11.7 / 10.4 |
| n=8192 simdgroup_matrix 8x8 fp16 -> fp32, 64x64/TG | 180.547 | 193.109 | 0.263 | 180.949 / 193.421 | 6.09 TOPS | 3.0 / 2.8 |
| n=8192 MPSMatrixMultiplication fp16 -> fp32 | 41.391 | 48.209 | 0.467 | 43.200 / 49.608 | 26.56 TOPS | 13.0 / 11.1 |

(bytes = A + B + C once; inputs are integers in [-2, 2], 128 sampled outputs checked exactly (fp16 output: 0.2% tolerance))


### G4c sustained: each MPP matmul at n = 4096 encoded 8 times per command buffer (warm)

#### G4a Metal 4 tensor ops (MPP matmul2d) square M=N=K; C = A*B  (load avg 78 115 126)

| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | TOPS (2MNK) (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| n=4096 MPP fp16 x fp16 -> fp32, 64x32 tile, 4 SG x8 per CB | 35.433 | 38.665 | 0.237 | 35.758 / 38.983 | 31.03 TOPS | 3.8 / 3.5 |
| n=4096 MPP fp16 x fp16 -> fp32, 128x64 tile, 4 SG x8 per CB | 34.356 | 42.752 | 0.239 | 34.842 / 48.205 | 32 TOPS | 3.9 / 3.1 |
| n=4096 MPP fp16 x fp16 -> fp32, 128x128 tile, 4 SG x8 per CB | 93.975 | 98.571 | 0.350 | 94.568 / 99.159 | 11.7 TOPS | 1.4 / 1.4 |
| n=4096 MPP fp16 x fp16 -> fp16, 64x32 tile, 4 SG x8 per CB | 112.318 | 113.194 | 0.284 | 113.602 / 113.692 | 9.789 TOPS | 0.9 / 0.9 |
| n=4096 MPP int8 x int8 -> int32, 64x32 tile, 4 SG x8 per CB | 50.034 | 50.844 | 0.244 | 50.384 / 51.200 | 21.98 TOPS | 2.0 / 2.0 |
| n=4096 MPP int8 x int8 -> int32, 128x64 tile, 4 SG x8 per CB | 25.694 | 27.442 | 0.250 | 26.661 / 27.806 | 42.79 TOPS | 3.9 / 3.7 |
| n=4096 MPP fp16 x fp16 -> fp32, 128x64 tile, 4 SG, swizzle 8 x8 per CB | 54.386 | 56.168 | 0.252 | 54.753 / 59.509 | 20.22 TOPS | 2.5 / 2.4 |
| n=4096 MPP int8 x int8 -> int32, 128x64 tile, 4 SG, swizzle 8 x8 per CB | 27.198 | 27.523 | 0.252 | 27.575 / 27.982 | 40.43 TOPS | 3.7 / 3.7 |
| n=4096 MPP int8 x int8 -> int32, 128x64 tile, 4 SG, swizzle 16 x8 per CB | 25.539 | 25.780 | 0.259 | 26.406 / 27.156 | 43.05 TOPS | 3.9 / 3.9 |
| n=4096 MPP int8 x int8 -> int32, 128x128 tile, 4 SG x8 per CB | 35.955 | 37.249 | 0.253 | 37.247 / 37.669 | 30.58 TOPS | 2.8 / 2.7 |
| n=4096 MPP int8 x int8 -> int32, 256x128 tile, 8 SG x8 per CB | 41.685 | 42.532 | 0.256 | 42.100 / 42.941 | 26.38 TOPS | 2.4 / 2.4 |


(In the x8 rows, TOPS counts all eight matmuls; the GB/s column counts one matmul's bytes and should be ignored.)

Reading: in bursts the matrix units deliver **106 to 110 TOPS int8** and **58 to 63 TOPS fp16 → fp32**
(n = 2048 to 4096, 128x64 tile, 4 SIMD-groups; this tile is the best of those tried). Apple's MPS fp16 is
in the same range (49 to 55 TOPS), and the simple `simdgroup_matrix` kernel gets 12 to 14 TOPS (4 to 5x
below MPP). fp16 output is no faster than fp32 output.

Sustained work drops to about **43 TOPS int8** and **32 TOPS fp16**. The x8 rows run the same n = 4096
kernel eight times, 26 to 35 ms in total, and lose 2.5x per kernel. The n = 8192 rows in G4a (30 to 90 ms
single dispatches) show the same drop, for MPS too. The swizzle recovers part of it at n = 8192 (38 → 51
TOPS int8), so cache locality is part of the story there. But G4c shows the main effect is a
sustained-clock limit, not the cache. DRAM bandwidth does not drop under sustained load (G2c). The
plausible cause is the shared SoC power or thermal budget while the CPUs are saturated (load average ~130).
This was not proven here.

### G4b one-hot shapes (warm)

C (R x N) = onehot(code) (R x 256, int8, materialised in memory) x B (256 x N, int8) → int32. That is a
row gather C[r] = B[code[r]], and is compared with a plain gather kernel that reads u16 codes.

#### G4b one-hot shapes: C (R x N) = onehot(code) (R x 256, int8) * B (256 x N, int8) -> int32  (load avg 111 128 132)

| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | rows/s (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| R=1048576, K=256, N=1: MPP int8 matmul, tile 128x8 | 0.636 | 0.645 | 0.176 | 0.904 / 0.935 | 1.649e+09 rows/s | 428.8 / 422.6 |
| R=1048576, K=256, N=1: MPP int8 matmul, tile 64x8 | 0.618 | 0.623 | 0.104 | 0.814 / 0.837 | 1.695e+09 rows/s | 440.8 / 437.9 |
| R=1048576, N=1: plain gather C[r] = B[code[r]] (u16 codes) | 0.032 | 0.032 | 0.117 | 0.204 / 0.238 | 3.251e+10 rows/s | 195.1 / 194.3 |
| R=1048576, K=256, N=8: MPP int8 matmul, tile 128x8 | 0.734 | 0.743 | 0.141 | 0.935 / 0.993 | 1.428e+09 rows/s | 411.3 / 406.7 |
| R=1048576, K=256, N=8: MPP int8 matmul, tile 64x8 | 0.704 | 0.710 | 0.128 | 0.903 / 0.916 | 1.489e+09 rows/s | 428.8 / 425.2 |
| R=1048576, N=8: plain gather C[r] = B[code[r]] (u16 codes) | 0.045 | 0.045 | 0.106 | 0.213 / 0.237 | 2.337e+10 rows/s | 794.5 / 792.3 |
| R=1048576, K=256, N=32: MPP int8 matmul, tile 64x32 | 0.896 | 0.899 | 0.137 | 1.089 / 1.234 | 1.171e+09 rows/s | 449.6 / 447.8 |
| R=1048576, K=256, N=32: MPP int8 matmul, tile 128x8 | 1.684 | 1.694 | 0.164 | 1.940 / 1.990 | 6.225e+08 rows/s | 239.1 / 237.7 |
| R=1048576, K=256, N=32: MPP int8 matmul, tile 64x8 | 1.669 | 1.681 | 0.190 | 1.963 / 2.102 | 6.281e+08 rows/s | 241.2 / 239.6 |
| R=1048576, N=32: plain gather C[r] = B[code[r]] (u16 codes) | 0.337 | 0.349 | 0.193 | 1.195 / 2.267 | 3.114e+09 rows/s | 404.9 / 390.3 |

load avg after G4: 120 130 132


Reading: the materialised one-hot matmul is **bandwidth-bound on the one-hot operand** (256 B per row, at
420 to 450 GB/s): 1.2 to 1.7 G rows/s. The plain gather reads 2 B per row and does 31 to 33 G rows/s at
N = 1, 20x faster. Tiles narrower than the output (N = 32 with TN = 8) halve throughput. Very small N
works with MPP (it pads internally) but gains nothing. Note: gather rates above 550 GB/s (N = 8) mean the
32 MB output is being absorbed by the system-level cache within the measured window.

## G5. Intersections

### G5a sorted u16 arrays, warm

P pairs of sorted, unique u16 arrays, both of length L, values with mean gap 4 (about 25% overlap). Each
side holds 16M elements in total (64 MB of data in all). Bytes = 4·L·P + output. Kernels: *thread/pair*
is a branchless merge by one thread. *SIMD-group / binary search*: each lane takes every 32nd element of A
and binary-searches B. *SIMD-group / chunk + merge*: each lane takes a contiguous 1/32 of A, finds its
start in B by binary search, then merges. All loops are bounded.

#### G5a sorted u16 array intersection counts (both sides length L, ~25% overlap; 2 x 16M elements total)  (load avg 128 129 132)

| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | pairs/s (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| L=16, P=1048576: thread/pair merge | 0.152 | 0.167 | 0.188 | 1.153 / 3.417 | 6.893e+09 pairs/s | 468.7 / 426.0 |
| L=16, P=1048576: SIMD-group/pair, per-element binary search | 0.461 | 0.463 | 0.145 | 0.658 / 0.910 | 2.273e+09 pairs/s | 154.6 / 154.0 |
| L=16, P=1048576: SIMD-group/pair, lane chunk + merge | 0.626 | 0.626 | 0.163 | 0.834 / 1.125 | 1.676e+09 pairs/s | 113.9 / 113.8 |
| L=64, P=262144: thread/pair merge | 0.173 | 0.189 | 0.219 | 0.957 / 12.267 | 1.513e+09 pairs/s | 393.4 / 360.9 |
| L=64, P=262144: SIMD-group/pair, per-element binary search | 0.246 | 0.248 | 0.149 | 0.452 / 0.934 | 1.066e+09 pairs/s | 277.3 / 275.4 |
| L=64, P=262144: SIMD-group/pair, lane chunk + merge | 0.253 | 0.254 | 0.115 | 0.457 / 0.493 | 1.035e+09 pairs/s | 269.2 / 268.6 |
| L=256, P=65536: thread/pair merge | 0.233 | 0.237 | 0.155 | 0.485 / 0.620 | 2.813e+08 pairs/s | 289.1 / 284.0 |
| L=256, P=65536: SIMD-group/pair, per-element binary search | 0.250 | 0.250 | 0.154 | 0.466 / 0.541 | 2.625e+08 pairs/s | 269.8 / 269.2 |
| L=256, P=65536: SIMD-group/pair, lane chunk + merge | 0.159 | 0.161 | 0.118 | 0.320 / 0.366 | 4.129e+08 pairs/s | 424.5 / 418.6 |
| L=1024, P=16384: thread/pair merge | 0.404 | 0.412 | 0.137 | 0.581 / 0.665 | 4.053e+07 pairs/s | 166.2 / 163.1 |
| L=1024, P=16384: SIMD-group/pair, per-element binary search | 0.301 | 0.301 | 0.136 | 0.518 / 0.548 | 5.443e+07 pairs/s | 223.2 / 222.8 |
| L=1024, P=16384: SIMD-group/pair, lane chunk + merge | 0.161 | 0.164 | 0.135 | 0.334 / 0.389 | 1.017e+08 pairs/s | 417.0 / 409.3 |
| L=4096, P=4096: thread/pair merge | 1.585 | 1.590 | 0.192 | 1.866 / 2.303 | 2.585e+06 pairs/s | 42.4 / 42.2 |
| L=4096, P=4096: SIMD-group/pair, per-element binary search | 0.388 | 0.389 | 0.131 | 0.575 / 0.635 | 1.054e+07 pairs/s | 172.8 / 172.7 |
| L=4096, P=4096: SIMD-group/pair, lane chunk + merge | 0.206 | 0.209 | 0.110 | 0.386 / 0.413 | 1.99e+07 pairs/s | 326.1 / 320.4 |


### G5b 8 KB bitmaps (65536 bits), SIMD-group per pair, warm

#### G5b bitmap AND + popcount, 8 KB (65536-bit) bitmaps, SIMD-group per pair  (load avg 137 131 132)

| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | pairs/s (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| P=16384 streaming disjoint pairs | 0.500 | 0.503 | 0.147 | 0.758 / 1.061 | 3.278e+07 pairs/s | 537.4 / 534.5 |
| P=65536 random pairs from 4096-bitmap pool (32 MB) | 0.764 | 0.765 | 0.178 | 0.998 / 1.060 | 8.574e+07 pairs/s | 1405.8 / 1403.8 |
| P=65536 random pairs from 512-bitmap pool (4 MB) | 0.351 | 0.353 | 0.157 | 0.560 / 1.243 | 1.866e+08 pairs/s | 3060.2 / 3048.3 |

(G5b bytes = 16 KB of bitmap reads per pair + indices + output, i.e. logical traffic, not unique bytes)

load avg after G5: 137 131 132


Reading: with the right mapping, sorted-array intersection runs at **330 to 470 GB/s**, i.e. close to DRAM
speed (16 to 64 MB working sets partly hit the system-level cache). That means thread-per-pair for
L ≤ 64, and SIMD-group chunk + merge for L ≥ 256. The best rates are 6.9 G pairs/s at L = 16, 0.41 G at
L = 256, 0.10 G at L = 1024, and 20 M at L = 4096. Thread-per-pair collapses at L = 4096 (42 GB/s:
divergent, uncoalesced). Bitmap AND+popcount of streaming pairs runs at DRAM speed (537 GB/s, 33 M
pairs/s). When pairs index a pool that fits the system-level cache (32 MB), it reaches 1.4 TB/s logical
(86 M pairs/s); for a 4 MB pool, 3 TB/s (187 M pairs/s). An 8 KB bitmap costs as many bytes per pair as a
sorted array of L = 4096 (density 1/16 of a 65536 universe). Below that density, sorted arrays move fewer
bytes; above it, bitmaps do, and bitmaps also gain from cache reuse when the same sets recur.

## G6. Segmented histogram (group-by count), warm

u16 codes < 1024. Segments of 2000 to 4000 elements (uniformly random, ~3,000 on average, like one
document's nodes). The output is a dense u32 histogram per segment (4 KB per segment, about 2/3 of the
input bytes). "Skewed": half the elements fall on 8 codes. Bytes = codes + offsets + output. Every count
was checked.

Kernels: *TG/segment*: one threadgroup per segment, 4 KB threadgroup atomic histogram. *SG/segment*: one
SIMD-group per segment, each with its own threadgroup histogram. *Device atomics*: atomics straight into the
output (zeroed by a blit in the same command buffer). *Matrix units*: code c = hi·32 + lo. Per chunk of 32
elements, two one-hot int8 tiles (hi x element, element x lo) are built in threadgroup memory, and
H (32 x 32) += L x R runs on MPP `matmul2d` (int8 → int32, `multiply_accumulate`, one SIMD-group, cooperative
destination tensor stored at the end).

#### G6 segmented histogram: u16 codes < 1024, segments 2000..4000 elements, dense u32 counts out  (load avg 110 125 130)

| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | Gelem/s (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| 10M elems, 3320 segs: TG/segment, tg atomics, 256 threads | 0.042 | 0.043 | 0.111 | 0.243 / 0.289 | 237.2 Gelem/s | 797.1 / 789.3 |
| 10M elems, 3320 segs: TG/segment, tg atomics, 1024 threads | 0.052 | 0.054 | 0.142 | 0.273 / 0.358 | 191.4 Gelem/s | 643.3 / 625.3 |
| 10M elems, 3320 segs: SG/segment, 4 seg/TG (16 KB tg) | 0.099 | 0.102 | 0.123 | 0.299 / 0.362 | 100.7 Gelem/s | 338.4 / 328.3 |
| 10M elems, 3320 segs: SG/segment, 2 seg/TG (8 KB tg) | 0.095 | 0.125 | 0.153 | 0.776 / 1.478 | 104.8 Gelem/s | 352.4 / 270.0 |
| 10M elems, 3320 segs: device atomics (+ blit zero of H) | 0.077 | 0.085 | 0.154 | 0.249 / 0.330 | 129.2 Gelem/s | 434.2 / 394.3 |
| 10M elems, 3320 segs: matrix units: one-hot hi x lo int8 matmul (MPP), SG/segment | 0.203 | 0.204 | 0.113 | 0.375 / 0.410 | 49.37 Gelem/s | 166.0 / 164.7 |
| 30M elems, 9969 segs: TG/segment, tg atomics, 256 threads | 0.177 | 0.189 | 0.219 | 3.269 / 5.818 | 169.1 Gelem/s | 568.7 / 533.0 |
| 30M elems, 9969 segs: TG/segment, tg atomics, 1024 threads | 0.210 | 0.215 | 0.153 | 0.455 / 0.635 | 142.8 Gelem/s | 480.1 / 470.0 |
| 30M elems, 9969 segs: SG/segment, 4 seg/TG (16 KB tg) | 0.383 | 0.386 | 0.155 | 0.630 / 0.681 | 78.36 Gelem/s | 263.5 / 261.5 |
| 30M elems, 9969 segs: SG/segment, 2 seg/TG (8 KB tg) | 0.366 | 0.376 | 0.200 | 1.018 / 2.733 | 82.03 Gelem/s | 275.8 / 268.2 |
| 30M elems, 9969 segs: device atomics (+ blit zero of H) | 0.329 | 0.337 | 0.179 | 0.547 / 0.754 | 91.28 Gelem/s | 306.9 / 299.1 |
| 30M elems, 9969 segs: matrix units: one-hot hi x lo int8 matmul (MPP), SG/segment | 0.549 | 0.551 | 0.205 | 0.806 / 1.270 | 54.65 Gelem/s | 183.8 / 183.0 |
| 30M elems, 9969 segs, skewed: TG/segment, tg atomics, 256 threads | 0.197 | 0.216 | 0.226 | 0.957 / 4.178 | 152.4 Gelem/s | 512.4 / 466.3 |
| 30M elems, 9969 segs, skewed: TG/segment, tg atomics, 1024 threads | 0.202 | 0.241 | 0.179 | 0.469 / 2.501 | 148.8 Gelem/s | 500.3 / 419.3 |
| 30M elems, 9969 segs, skewed: SG/segment, 4 seg/TG (16 KB tg) | 0.383 | 0.388 | 0.162 | 0.629 / 1.184 | 78.35 Gelem/s | 263.5 / 260.2 |
| 30M elems, 9969 segs, skewed: SG/segment, 2 seg/TG (8 KB tg) | 0.360 | 0.494 | 0.194 | 0.679 / 4.477 | 83.45 Gelem/s | 280.6 / 204.1 |
| 30M elems, 9969 segs, skewed: device atomics (+ blit zero of H) | 0.332 | 0.335 | 0.178 | 0.582 / 0.658 | 90.43 Gelem/s | 304.1 / 301.4 |
| 30M elems, 9969 segs, skewed: matrix units: one-hot hi x lo int8 matmul (MPP), SG/segment | 0.534 | 0.551 | 0.170 | 0.803 / 0.921 | 56.23 Gelem/s | 189.1 / 183.2 |

(bytes = codes (2 B/elem) + offsets + dense output (4 KB/segment); all counts checked)

load avg after G6: 114 126 130


Reading: threadgroup atomics with 256 threads per segment are best: **150 to 170 Gelem/s on 30M
elements** (510 to 570 GB/s, i.e. DRAM-bound), and 237 Gelem/s on 10M, whose ~34 MB working set sits in the
system-level cache. Skew costs little (169 → 152 Gelem/s). Per-SIMD-group histograms are 2x slower (fewer
threads per segment, more threadgroup memory). Device atomics are 1.8x slower. The matrix-unit
formulation is correct, and it saturates the matrix units: 55 Gelem/s x 1024 MACs per element ≈ 112 int8
TOPS, the G4 burst peak. It is still **3x slower** than atomics, because a one-hot product spends 1024
multiply-adds on each element that an atomic counts with one add.

## What this means for relational work

1. **Bandwidth is the ceiling for nearly everything relational.** DRAM is ~550 GB/s and stays there under
   sustained load. Intersections, histograms, gathers and materialised one-hot products all land at 400 to
   570 GB/s once mapped sensibly. Anything that fits in about 32 MB gets roughly 1.7x (900+ GB/s), and
   sets reused from a 4 MB pool get more. So the design lever is **bytes per fact**: dense u16 codes,
   sorted u16 runs or 8 KB bitmaps chosen by density, and working sets kept under ~32 MB.
2. **The matrix units are real and reachable** (Metal 4 MPP from run-time MSL, no offline compiler):
   ~110 TOPS int8 and ~60 TOPS fp16 in bursts, ~43 / ~32 TOPS sustained on this loaded machine. A
   kernel only benefits if it does more than ~200 int8 ops per DRAM byte in bursts (~80 sustained).
   Dense count-weighted products (e.g. join-count matrices between two code sets, or counts x weights)
   qualify. Boolean work mostly does not:
   - **Boolean products**: bit-packed AND+popcount on the plain ALUs does ~70 Tbit-op/s, more than the
     int8 units' ~55 T multiply-adds/s on 0/1 bytes, and with 8x fewer bytes. For boolean matrices,
     pack bits and use popcount, not the matrix units.
   - **Never materialise one-hot matrices in memory.** That is 128x the bytes of u16 codes, and it ran
     20x slower than a gather (G4b).
   - **Even one-hot built in registers loses to atomics** for group-by count (55 vs 150+ Gelem/s). The
     matrix units fit group-by only when the per-element work is genuinely a dense product (many value
     columns per row, e.g. summing 32+ int8 features per group).
3. **Launch and host costs dictate batching.** Device cost per dispatch is microseconds, but a host round
   trip is 0.13 ms at best and over 1 ms cold under load, plus 0.1 to 0.3 ms of host CPU each. A wave
   should be one command buffer with many dispatches (a concurrent encoder where they are independent,
   ~0.1 µs each), carrying at least about 1 ms of device work (roughly 0.5 GB of traffic) to amortise
   the round trip. Many small command buffers in flight reach 13k to 30k per second.
4. **Per-pair work maps differently by size.** Small sets (L ≤ 64) are best one per thread. Mid and large
   sets (L ≥ 256) are best one per SIMD-group with chunked merge. Both run close to bandwidth speed. Data
   layout (pairs contiguous, coalesced) matters more than the algorithm.

### Honest limits

- The machine was heavily loaded (load average 80 to 155) and shared with other sessions. Device time is
  robust to CPU load, but GPU clocks were not: cold vs warm differs up to 5x, and sustained compute runs
  about 2.5x below burst. Run-to-run variance on long compute kernels was up to 2x. GPU contention from
  other processes cannot be ruled out. Wall-time medians are not representative of an idle machine.
- G3 and G4a verify sampled outputs (256 and 128 per case), not every output. Matmul inputs are small
  integers, so fp16 accuracy on real data was not assessed.
- The MPP kernels use the documented tile structure with operands read straight from device memory. Tile
  shapes are a small sweep (64x32 to 256x128). No threadgroup staging, no int4, no bfloat16. The
  `MTLTensor` resource path and Metal 4 command queues (`MTL4CommandQueue`) were not tried; the classic
  `MTLCommandQueue` was used throughout.
- The `simdgroup_matrix` kernel is a straightforward one (no threadgroup staging or double buffering), so
  it understates that path. A tuned version would sit somewhere between it and MPS.
- G6's matrix-unit histogram is one formulation (32 x 32 hi/lo split, one SIMD-group per segment);
  others were not explored.
- Sorted-array data has a fixed L per run and uniform gaps. Real skewed length distributions would need
  load balancing that was not tested.
- "GB/s" for pooled bitmaps (G5b) and some gathers is logical traffic, partly served from cache, not DRAM
  traffic.

## Real workloads (data-layer corpus, 13,088 Python documents)

### Intro

The same harness, run on five workloads extracted from the corpus by `tools/matrix-offload/extract`. The
formats and semantics come from `extract/src/bin/baseline.rs`. Each GPU result is checked **exactly**
before it is timed: every output cell (W1 counts matrix, W5 upper triangle, W2 per-document counts, W3
closure matrix, W4 per-document histograms), plus the expected totals:

- W5: Σ = 29,397,646.
- W1: 1,473,222 nonzero cells.
- W2: 83,066,745 pairs.
- W3: 29,722,613 document reach pairs.
- W4: Σ = 22,295,712.

Outputs are poisoned before every verified run, so stale results cannot pass. The CPU baselines are the
single-thread CPU times in each `wN/baseline.txt`. Inputs are the corpus files mapped into shared buffers
with no copy (see "Inputs" below). All runs are warm (`GPUPROBE_WARM_MS=100`), at load average 21 to 90
(lower than for G1 to G6).

### Summary table

"bytes moved" counts inputs read once plus outputs written once. "vs CPU" compares the device time with the
named single-thread CPU form.

| workload | GPU form | device ms (min / med) | host CPU ms | bytes moved | GB/s | vs CPU baseline |
|---|---|---|---|---|---|---|
| W5 name co-occurrence | bit gram, upper-triangle tiles only, u16 out | 0.379 / 0.380 | 0.11 | 7.6 MB | 20 (compute-bound, 72 Tbit-op/s) | bitmap 125 ms → **330x**; merge 3.4 s → 9,000x |
| W1 prefilter overlap | dense bit gram 13088 x 444, u8 counts out | 0.077 / 0.077 | 0.09 | 7.1 MB | 92 | postings 10.3 ms → **134x**; dense 26 ms → 335x |
| W2 equality join, counts | (b) direct-indexed, threadgroup per doc, 2764-entry table | 0.082 / 0.083 | 0.11 | 11.9 MB | 144 | direct 2.7 ms → **33x**; hash 73 ms → 890x |
| W2 equality join, counts | (a) brute-force tiles (4.14 G compares, 1.9 T/s) | 2.148 / 2.237 | 0.13 | 12.2 MB | 6 | brute 824 ms → 384x; but 26x slower than (b) |
| W2 equality join, rows | (c) count + scan + scatter, 83 M rows (664.5 MB out) | 6.383 / 6.475 | 0.18 | 694 MB | **109** | hash emit rows 251 ms → **39x** |
| W3 import closure | (b) level-wise, 35 dispatches (one per DAG level) | 1.052 / 1.069 | 0.14 | 29.6 MB | 28 | bit-row DAG 3.1 ms → **3x** |
| W3 import closure | (a) boolean squaring, 6 steps in one CB, GPU-side fixpoint | 12.07 / 12.56 | 0.19 | 177 MB (21.2 GB of row reads) | 15 (1.75 TB/s logical, cache-served) | squaring 1.56 s → 129x; semi-naive ~5.4 s → 450x |
| W4 kind histogram | threadgroup per doc, tg atomics, 256 threads | 0.229 / 0.232 | 0.10 | 50.7 MB | 222 | direct 20.2 ms → **88x**; hash 375 ms → 1,600x |
| **wave** W1+W5+W4+W3(b)+W2(b) | one command buffer | 1.92 / 1.94 | 0.15 to 0.18 | ~107 MB | | sum of best CPU forms 161 ms → 84x |

### Raw tables

#### Real workloads W1, W3, W4, W5, W2 (a, b): each verified exactly, then timed  (load avg 25 67 91)

| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | - (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| W1 dense bit gram 13088x444, u8 counts out (verified) | 0.077 | 0.077 | 0.089 | 0.234 / 0.250 | | 92.3 / 92.2 |
| W5 bit gram upper triangle, 64x64 tiles, u16 counts out (verified) | 0.379 | 0.380 | 0.113 | 0.537 / 0.581 | | 19.9 / 19.9 |
| W4 TG/doc, one tg atomic histogram, 256 threads (verified) | 0.229 | 0.232 | 0.101 | 0.405 / 0.411 | | 221.7 / 218.9 |
| W4 TG/doc, 8 per-SIMD-group histograms, 256 threads (verified) | 0.236 | 0.238 | 0.084 | 0.398 / 0.428 | | 214.7 / 212.7 |
| W3 (a) boolean squaring, row-OR, 6 steps in one CB, GPU-side fixpoint flag (verified) | 12.068 | 12.556 | 0.187 | 12.349 / 12.832 | | 14.6 / 14.1 |
| W3 (b) level-wise (35 dispatches, one per DAG level) (verified) | 1.052 | 1.069 | 0.137 | 1.232 / 1.283 | | 28.1 / 27.7 |
| W2 (b) direct-indexed counts, TG/doc, 2764-entry tg table (verified) | 0.082 | 0.083 | 0.107 | 0.225 / 0.261 | | 143.9 / 143.5 |
| W2 (a) brute-force tiles, 4.14 G compares (verified) | 2.148 | 2.237 | 0.131 | 2.345 / 2.473 | | 5.7 / 5.5 |

##### Summary (load avg 25 67 91)

| workload | form | device ms min / med | host CPU ms med | bytes moved | GB/s at min | CPU baseline (1 thread) → speed-up |
|---|---|---|---|---|---|---|
| W1 | dense bit gram 13088x444, u8 counts out | 0.077 / 0.077 | 0.089 | 7.1 MB | 92 | postings 10.3 ms → 134x |
| W5 | bit gram upper triangle, 64x64 tiles, u16 counts out | 0.379 / 0.380 | 0.113 | 7.6 MB | 20 | bitmap 125.0 ms → 330x |
| W4 | TG/doc, one tg atomic histogram, 256 threads | 0.229 / 0.232 | 0.101 | 50.7 MB | 222 | direct 20.2 ms → 88x |
| W4 | TG/doc, 8 per-SIMD-group histograms, 256 threads | 0.236 / 0.238 | 0.084 | 50.7 MB | 215 | direct 20.2 ms → 86x |
| W3 | (a) boolean squaring, row-OR, 6 steps in one CB, GPU-side fixpoint flag | 12.068 / 12.556 | 0.187 | 176.6 MB | 15 | squaring 1556.7 ms → 129x |
| W3 | (b) level-wise (35 dispatches, one per DAG level) | 1.052 / 1.069 | 0.137 | 29.6 MB | 28 | bit-row DAG 3.1 ms → 3x |
| W2 | (b) direct-indexed counts, TG/doc, 2764-entry tg table | 0.082 / 0.083 | 0.107 | 11.9 MB | 144 | direct 2.7 ms → 33x |
| W2 | (a) brute-force tiles, 4.14 G compares | 2.148 / 2.237 | 0.131 | 12.2 MB | 6 | brute 824.0 ms → 384x |

#### Wave framing: W1 + W5 + W4 + W3(b) + W2(b)  (load avg 25 67 91)

| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | - (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| 5 command buffers, each committed and waited (dev = sum) | 1.931 | 2.000 | 0.520 | 2.958 / 3.034 | | |
| 1 command buffer, one serial encoder per workload (all verified) | 1.981 | 2.013 | 0.148 | 2.216 / 2.257 | | |
| 1 command buffer, one concurrent encoder (barriers only inside W1/W5/W3) (all verified) | 1.915 | 1.936 | 0.181 | 2.185 / 2.256 | | |

load avg after real workloads: 25 67 91


#### W2 (c): materialise the (read, binding) rows  (load avg 21 62 89)

| case | dev ms min | dev ms med | host CPU ms med | wall ms min / med | - (at min) | GB/s min / med |
|---|---|---|---|---|---|---|
| W2 (c) emit 83 M (read, binding) rows: count + scan + scatter (verified) | 6.383 | 6.475 | 0.178 | 6.599 / 6.708 | | 108.8 / 107.2 |

##### Summary (load avg 21 62 89)

| workload | form | device ms min / med | host CPU ms med | bytes moved | GB/s at min | CPU baseline (1 thread) → speed-up |
|---|---|---|---|---|---|---|
| W2 | (c) emit 83 M (read, binding) rows: count + scan + scatter | 6.383 / 6.475 | 0.178 | 694.4 MB | 109 | hash emit rows 251.2 ms → 39x |


### Inputs: copy, no-copy, private

#### Input residency: 10 files, 75.9 MB (page cache warm)  (load avg 21 62 89)

| path | host CPU ms (min of 5) | wall ms (min of 5) | device ms | note |
|---|---|---|---|---|
| read file + makeBuffer(bytes:) (copy into a new shared buffer) | 10.73 | 10.92 | | |
| makeBuffer(length:) + read(2) straight into contents (one copy) | 7.61 | 7.62 | | |
| mmap + makeBuffer(bytesNoCopy:) (no copy) | 0.25 | 0.25 | | |
| W4 kernel on a fresh no-copy mapping of kinds.bin: first run vs later | 8.51 | 8.55 | 0.267 first / 0.232 later | first-touch cost of mapped pages |
| blit all inputs shared → private | 0.12 | 1.25 | 0.284 | 536 GB/s (read + write) |


Reading:

- **Shared storage avoids the copy, not the cost.** Wrapping the mapped files with
  `makeBuffer(bytesNoCopy:)` costs 0.25 ms of host CPU for all 76 MB. Copying instead (read plus
  `makeBuffer(bytes:)`, or read(2) straight into a shared buffer) costs 7.6 to 10.7 ms of CPU, at 7 to 10
  GB/s. But the first command buffer that touches a fresh mapping pays the page faults and wiring on the
  host: 8.5 ms of CPU for kinds.bin (44.7 MB), about as much as copying it. Device time is unchanged
  (0.267 vs 0.232 ms).
- So the input cost is roughly **one host pass over the bytes (about 5 to 10 GB/s of CPU), paid once per
  residency**, either as a copy or as first-touch faults. Data the GPU keeps resident across waves pays it
  once.
- Private storage is not needed: a blit of everything to private memory runs at 536 GB/s (0.28 ms). G2
  showed private reads are no faster than shared.

### What the real workloads say

1. **The GPU beats the best single-thread CPU form on every workload, by 3x to 330x in device time.** The
   largest gains are on the dense restatements, which are compute-bound on the plain ALUs (bit-packed
   popcount): W5 at 72 Tbit-op/s (330x the CPU bitmap form) and W1 (134x the engine-style postings). W4
   and W2 counts gain 33x to 88x. Every one of them is far below a millisecond, so at this corpus size
   **the host round trip (0.13 ms at best, ~1 ms cold) and the per-command-buffer host CPU (~0.1 to
   0.2 ms) are the same size as the work.**
2. **Brute force pays only when it is also the right algorithm.** For W2 counts, the direct-indexed table
   (b) beats the all-pairs tiles (a) by 26x on the GPU, as it does on the CPU (2.7 ms vs 824 ms).
   Brute-force tiles make sense for keys that cannot be direct-indexed.
3. **Materialising join rows is a write-bandwidth job.** 83 M rows (664.5 MB) take 6.4 ms, 39x the CPU
   hash emit, but at only 109 GB/s, about 1/5 of DRAM speed. Each read writes its run of rows serially,
   which is uncoalesced. A SIMD-cooperative or sorted write should approach ~500 GB/s (about 1.4 ms).
   This was not done here. At any rate, 664 MB per wave is a big memory bill: prefer counts or semi-join
   bitmaps where the consumer allows it.
4. **Closure: a level-wise wavefront in one command buffer beats squaring by 11x.** But it is only 3x the
   CPU's bit-row DAG closure. Its 35 levels (the longest path) cost 35 dependent dispatches, ~30 µs
   each, set by the highest-out-degree row in each level. Squaring does 21 GB of row-ORs out of cache in
   12 ms. Its fixpoint test ran on the GPU (a changed flag per step; steps after convergence just copy),
   so no host round trip happens per step.
5. **Batching into one wave works.** All five (one form each) run in one command buffer in 1.9 ms of
   device time and 2.2 ms wall, with 0.15 to 0.18 ms of host CPU. Five separate command buffers cost
   about 0.8 ms more wall and 3x the host CPU (0.45 to 0.65 ms). A concurrent encoder gained nothing
   measurable over serial encoders here (1.92 vs 1.98 ms), because W3's dependent levels dominate.
6. **Bandwidth is not the limit at this scale.** Only W4 (222 GB/s) and W2 (c) (109 GB/s) move enough
   bytes to matter, and both sit below the 550 GB/s ceiling. W4 is likely limited by threadgroup-atomic
   contention on a few dominant kinds; per-SIMD-group sub-histograms did not help (215 GB/s).

Limits: one corpus, warm GPU, page cache warm. Device time excludes the first-touch host cost, which is
listed separately. W3 (b) takes its level list from the host (0.14 ms CPU). W2 (a) takes a tile list from
the host (21,245 tiles, 0.7 ms CPU). Both could be built on the GPU and were not.
