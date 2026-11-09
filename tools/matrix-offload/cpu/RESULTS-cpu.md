# CPU matrix-unit (SME/SME2) and NEON probes: results

Machine: Apple M5 Max, macOS 26.4.1, Apple clang 21, `-O2 -mcpu=apple-m4+cssc`. 18 cores in three
6-core clusters: one P ("Super", `hw.perflevel0`, logical CPUs 12-17, 16 MB L2) and two M
("Performance", `hw.perflevel1`, CPUs 0-5 and 6-11, 8 MB L2 each). There are no efficiency cores.
Streaming vector length is 512 bits (`svcntw() = 16`).

How the numbers were taken:

- Every timed figure is CPU time on the thread CPU clock (`CLOCK_THREAD_CPUTIME_ID`), or
  `getrusage` user+sys for whole multi-thread runs. Never wall time.
- Minimum over repetitions, with the median next to it where the two differ.
- Setup (packing, layout transforms, popcount metadata) is outside the timed region and listed
  separately.
- Every kernel's output was checked against a scalar reference before it was timed. Every check
  passed.
- Each table carries the load average and a clock estimate taken from a chain of dependent 1-cycle
  adds. Threads ask for `QOS_CLASS_USER_INTERACTIVE`, so single-thread runs land on P cores (each
  row names the core type of its fastest repetition).
- **The clock moves with machine load.** The final tables come from a pass at load average 28-40,
  where P cores ran at about 2.6-3.2 GHz. An earlier pass at load 110-167 measured 15-25 % lower
  SME throughput at the same cycle counts, with the P clock estimated at 2.58 GHz. Cycle counts
  (ns × GHz) are the stable quantity; absolute GB/s and TOPS swing with the clock.

Raw outputs: `<scratchpad>/cpu-runs/*.txt` (final pass) and `cpu-runs/load130/` (loaded pass).

## Headline numbers (one P core unless stated)

| quantity | value |
|---|---|
| smstart+smstop with ZA (compiler-generated, incl. d8-d15 spills) | 19.5 ns ≈ 62 outer products |
| outer products per streaming block for ≥ 90 % of peak | ≈ 1,000 (4,096 for ~100 %) |
| FMOPA fp32 / BMOPA issue rate (4 tiles) | 1 per cycle: 1.47 TFLOPS fp32, 26 Tbit-op/s BMOPA at ~3.2 GHz |
| int8 SMOPA/UMOPA, fp16/bf16 widening FMOPA, int16 SMOPA | 1 per 2 cycles: 3.25 TOPS int8, 1.6 TFLOPS fp16 |
| SME unit sharing | **one SME unit per cluster**, time-shared by that cluster's cores |
| SME per cluster | P: ~1.3 TFLOPS fp32 / ~20-23 Tbit-op/s; each M: ~0.45 TFLOPS / ~7.5 Tbit-op/s |
| bit-matrix AND-popcount, k ≥ 16 Kbit | SME 18-20 Tbit-op/s vs NEON 0.47 vs scalar 0.21: SME is 40-50x NEON |
| streaming-load bandwidth (SME) from L2 / DRAM, 1 thread | 150-160 / 100-106 GB/s |
| NEON load bandwidth from L1 / L2 / DRAM, 1 thread | 108 / 66-81 / 60 GB/s |
| DRAM, all threads (NEON) | ~30 GB/s per thread at 6 concurrent, ~18 GB/s at 12 concurrent: ≈ 180-220 GB/s aggregate, shared with other tenants |

## C1. Streaming-mode entry/exit

Load 39.8, clock estimate 3.18 GHz, P core. Each row is a `noinline` function called 200,000 times;
times are per call.

| entry/exit variant | ns/call min | median |
|---|---|---|
| plain noinline call | 0.63 | 0.63 |
| `__arm_locally_streaming` (smstart sm / smstop sm + d8-d15 spills) | 11.95 | 11.95 |
| locally_streaming + `__arm_new("za")` + zero za | 19.50 | 19.50 |
| `__arm_new("za")` only (smstart za, not streaming) + zero za | 7.55 | 7.55 |
| raw asm `smstart sm; smstop sm` | 11.64 | 11.64 |
| raw asm `smstart; smstop` (sm+za) | 11.64 | 11.64 |

Amortisation: K outer products per streaming block (round-robin over 4 tiles, operands in
registers, one row stored at the end). Each block enters and leaves streaming mode with ZA.

| K | FMOPA ns/block | ns/MOPA | BMOPA ns/block | ns/MOPA |
|---|---|---|---|---|
| 4 | 42.9 | 10.72 | 44.8 | 11.20 |
| 16 | 78.4 | 4.90 | 48.2 | 3.01 |
| 64 | 97.7 | 1.53 | 63.1 | 0.99 |
| 128 | 117.2 | 0.92 | 79.1 | 0.62 |
| 256 | 140.8 | 0.55 | 141.4 | 0.55 |
| 512 | 205.6 | 0.40 | 191.0 | 0.37 |
| 1024 | 343.7 | 0.34 | 362.0 | 0.35 |
| 4096 | 1235.2 | 0.30 | 1333.3 | 0.33 |
| 65536 | 20661.9 | 0.315 | 20661.3 | 0.315 |

- Entering and leaving streaming mode costs about 12 ns, and about 20 ns once ZA comes in. In
  outer products that is ~60 MOPAs.
- 1,000 outer products per block reach 90 % of peak; 4,096 reach about 100 %.
- Surprise: at small K, a FMOPA block costs ~80-120 ns even for K = 16-128, while a BMOPA block
  grows smoothly from 45 ns. A guess, not verified: the floating-point path pays a warm-up after
  `smstart`. Either way, short FP streaming blocks are worse than the entry cost alone suggests.
- Design consequence: a kernel should enter streaming mode once per large unit of work (a panel
  row, a group of 64 bitmaps), never once per tile or per container.

## C2. BMOPA semantics and throughput

Load 39.8, clock estimate 2.9-3.2 GHz, P core.

**Semantics (verified):** `svbmopa_za32_u32_m` computes
`ZA[i][j] += popcount(~(zn[i] ^ zm[j]))`. This is the XNOR-popcount outer product over 32-bit
lanes: 16 × 16 cells × 32 bits = 8,192 bit-ops per instruction.

| inputs (all lanes) | ZA[0][0] | popcount(~(a^b)) | popcount(a&b) |
|---|---|---|---|
| a=0, b=0 | 32 | 32 | 0 |
| a=~0, b=0 | 0 | 0 | 0 |
| a=~0, b=~0 | 32 | 32 | 32 |
| a=F0F0F0F0, b=FF00FF00 | 16 | 16 | 8 |
| a=1, b=3 | 31 | 31 | 1 |

Further checks, all passing:

- Random 16 × 16, accumulated 3 times: 256/256 cells match.
- Predicated run (rows 0-7, odd columns active): 256/256 inactive cells are left unchanged.
- `svbmops` subtracts the same quantity (it wraps modulo 2^32).

**AND-popcount from BMOPA** (verified on 16 × 16 × 2,048 bits, 256/256 cells):

    pop(a & b) = (pop(a) + pop(b) - pop(a ^ b)) / 2,   pop(a ^ b) = 32K - xnor
    C[i][j]   = (popA_i + popB_j - 32K + xnor_ij) / 2          (K = 32-bit words)

The row and column corrections need no extra pass. SME's `ADDVA`/`ADDHA` add a vector to every
column or row of a tile. Seeding ZA with `popA_i - 32K` (ADDVA) and `popB_j` (ADDHA) before the
accumulation leaves `2 × C[i][j]` in ZA, so the readout only shifts right by 1 (see C4).

**Throughput** (K = 65,536 BMOPAs per streaming block):

| variant | ns/BMOPA | Tbit-op per CPU-s | operand GB/CPU-s |
|---|---|---|---|
| registers, 1 tile | 1.391 | 5.9 | - |
| registers, 2 tiles | 0.696 | 11.8 | - |
| registers, 4 tiles | 0.316-0.349 | 23.5-25.9 | - |
| same, operands all-zero / all-ones / random | 0.316 each | 25.9 | - |
| 2x2 tiles, 4 loads per 4 BMOPA, 4 KB-64 KB (L1) | 0.349 | 23.5 | 183 |
| 2x2 tiles, one `ld1w {4 regs}` per 4 BMOPA, 1 MB (L2) | 0.350 | 23.4 | 183 |
| 2x2 tiles, 4 loads, 4 MB (L2) | 0.380 | 21.5 | 168 |
| 1x4 tiles (5 loads per 4 BMOPA), 4 MB (L2) | 0.398 | 20.6 | 201 |

- BMOPA issues at 1 per cycle; latency is about 4 cycles, so 4 tiles are needed to keep it busy.
- The two figures for "registers, 4 tiles" come from the same run: the clock stepped from about
  2.9 to about 3.2 GHz partway through. Timing does not depend on the data.
- The SME load path feeds a full-rate 2x2 block (64 B per BMOPA) from L1 and from a 1 MB L2
  working set. At 4 MB it loses 8-15 %.

## C3. Outer-product throughput by type and thread scaling

Core counts: `hw.perflevel0.physicalcpu` = 6 (P/"Super"), `hw.perflevel1.physicalcpu` = 12 (M),
`hw.ncpu` = 18, 6 CPUs per L2 in both levels.

Single thread, operands in registers, all tiles. Load 39.8, clock estimate 3.18 GHz, P core.

| instruction | ops/instr | ns/instr | ≈ cycles | G-ops per CPU-s |
|---|---|---|---|---|
| FMOPA fp32 (za32, 4 tiles) | 512 flop | 0.348 | 1.1 | 1,471 GFLOP |
| FMOPA fp16→fp32 widening | 1,024 flop | 0.630 | 2 | 1,625 GFLOP |
| BFMOPA bf16→fp32 | 1,024 flop | 0.630 | 2 | 1,625 GFLOP |
| SMOPA int8→int32 | 2,048 op | 0.630 | 2 | 3,251 GOP |
| UMOPA uint8→int32 | 2,048 op | 0.630 | 2 | 3,252 GOP |
| SMOPA int16→int32 (SME2) | 1,024 op | 0.630 | 2 | 1,626 GOP |
| BMOPA u32 XNOR-popcount | 8,192 bit-op | 0.315 | 1 | 25,971 Gbit-op |
| FMOPA fp64 (za64, 8 tiles) | 128 flop | 0.316 | 1 | 406 GFLOP |
| SMOPA int16→int64 (za64, 8 tiles) | 512 op | 0.316 | 1 | 1,622 GOP |
| NEON FMLA fp32 (control, 12 chains) | 8 flop | 0.121 | - | 66 GFLOP |

At the published M4 rates (~2 TFLOPS fp32, ~4 TOPS int8), the per-cycle rates match. The absolute
numbers here are lower because this loaded machine runs P cores at 2.6-3.2 GHz.

**Scaling.** How the measurement works:

- Each thread repeats one streaming block of 32,768 instructions and records the wall span, CPU
  time and CPU number of every block.
- A block counts as "clean" if it was on-CPU for ≥ 95 % of its span. For each clean block the tool
  counts the other threads whose clean blocks cover ≥ 90 % of that span; this is the observed
  concurrency.
- The table groups clean blocks by core type and by how many of the concurrent threads were on the
  same cluster. This holds up under load, where "T threads" does not mean T threads at once.

Load 28-40. Per-thread G-ops per CPU-s:

| instruction | P core: 1 / 2 / 3 / 4 / 5 / 6 threads on its cluster | M core: 1 / 2 / 3 / 4 / 5 / 6 on its cluster |
|---|---|---|
| FMOPA fp32 | 1270 / 664 / 442 / 331 / 265 / 221 | 465 / 237 / 158 / 118 / 98 / 83 |
| FMOPA fp16→fp32 | 1443 / 663 / 427 / 331 / 265 / 221 | 491 / 242 / 158 / 123 / 99 / 83 |
| SMOPA int8 | 2589 / 1229 / 784 / 661 / 529 / 441 | 982 / 496 / 330 / 248 / 198 / 160 |
| BMOPA (Gbit-op) | 20324 / 8948 / 7026 / 5311 / 4716 / 3938 | 7790 / 3974 / 2648 / 1986 / 1588 / 1323 |
| NEON FMLA control | 57 / 57 / 57 / 57 / 57 / 57 | 86 / 86 / 86 / 86 / 87 / 87 |

(These figures are from the 12-thread run. The 18-thread runs read the same way, with noisier
attribution.)

- **SME throughput is per cluster, not per core.** With q threads on a cluster, each gets 1/q:
  1270 → 664 → 442 → 331 → 265 → 221 adds up to about 1.3 TFLOPS per P cluster at any q.
- The NEON control scales perfectly per core, so the drop above is not a measurement artefact.
- The M clusters' SME units run at about 1/3 of the P cluster's rate (~0.45 TFLOPS fp32, ~1 TOPS
  int8, ~7.5 Tbit-op/s each).
- Chip total: about 1.3 + 2 × 0.45 ≈ 2.2 TFLOPS fp32, or about 35 Tbit-op/s BMOPA, using one SME
  thread per cluster.
- **CPU-time consequence:** a core waiting for its cluster's SME unit still burns CPU time. Six SME
  threads on one cluster spend about 5/6 of their CPU-seconds stalled, so the useful setting is
  one SME thread per cluster, with the other cores on NEON or scalar work.
- On this loaded machine, NEON fp32 ran faster on M cores (86 GFLOPS) than on P cores (57). That
  matches the M clusters clocking higher at the time.

## C4. Bit-matrix AND-popcount product C[i][j] = popcount(A_i & B_j)

A: n × k bits and B: m × k bits, both random (densities 1/4 and 1/2); C is n × m u32, written out.
Load 31.9-33.8, clock estimate 2.6-2.9 GHz, P core.

- **Scalar:** 2x2 register block, auto-vectorisation disabled. Two popcount forms: CSSC scalar
  `CNT`, and in parentheses fmov + `cnt.8b` + `addv`.
- **NEON:** 4x4 register block with `vandq` + `vcntq` + `vpadalq` into u16 accumulators. k is
  chunked to 8 Kbit, so the B chunk (m × 1 KB ≤ 8 MB) stays in L2.
- **SME:** A and B are packed into 32-row panels (`Ap[ib][word][32]`).
  - The kernel uses 2x2 tiles (a 32 × 32 output block): two `ld1w {2 regs}` loads feed 4 BMOPAs
    per 32-bit word.
  - The corrections are seeded into ZA with ADDVA/ADDHA.
  - Readout uses SME2 `MOVA` of 4 rows at a time, a multi-vector rounding shift right by 1
    (`URSHL` by -1; exact because every value is even), and `ST1W`.
  - B panels are grouped into ~4 MB sets that stay in L2 while all A panels stream past.
  - There is one streaming entry per (A panel, B group).
- Scalar and NEON compute a subset of rows (whole width) to stay inside the run-time cap. SME
  computes the whole product.

Columns:

- Gbit-op = n·m·k (or rows·m·k for the subsets) per CPU-second.
- "Operand loads" = bytes moved by SME load instructions.
- "Output writes" = C bytes per CPU-second.

| n=m | k bits | scalar CNT (NEON-cnt) | NEON 4x4 | SME BMOPA | SME CPU s (whole product) | SME pack ms | SME operand loads GB/CPU-s | SME output GB/CPU-s | SME / NEON |
|---|---|---|---|---|---|---|---|---|---|
| 1024 | 1024 | 179 (222) | 432 | 8,030 | 0.00013 | 0.1 | 63 | 31.4 | 19x |
| 1024 | 4096 | 190 (227) | 471 | 16,006 | 0.0003 | 0.2 | 125 | 15.6 | 34x |
| 1024 | 16384 | 174 (216) | 478 | 20,218 | 0.0008 | 0.9 | 158 | 4.9 | 42x |
| 1024 | 65536 | 174 (214) | 485 | 20,275 | 0.0034 | 4.1 | 158 | 1.2 | 42x |
| 2048 | 1024 | 180 (222) | 425 | 8,708 | 0.0005 | 0.1 | 68 | 34.0 | 20x |
| 2048 | 4096 | 172 (211) | 471 | 15,054 | 0.0011 | 0.4 | 118 | 14.7 | 32x |
| 2048 | 16384 | 168 (210) | 478 | 19,360 | 0.0035 | 1.7 | 151 | 4.7 | 41x |
| 2048 | 65536 | 174 (214) | 445 | 19,638 | 0.0140 | 8.1 | 153 | 1.2 | 44x |
| 4096 | 1024 | 180 (222) | 428 | 6,602 | 0.0026 | 0.2 | 52 | 25.8 | 15x |
| 4096 | 4096 | 172 (211) | 469 | 13,595 | 0.0051 | 0.9 | 106 | 13.3 | 29x |
| 4096 | 16384 | 168 (209) | 473 | 18,250 | 0.0151 | 3.2 | 143 | 4.5 | 39x |
| 4096 | 65536 | 174 (215) | 432 | 19,985 | 0.0550 | 15.7 | 156 | 1.2 | 46x |
| 8192 | 1024 | 180 (222) | 488 | 6,089 | 0.0113 | 0.4 | 48 | 23.8 | 12x |
| 8192 | 4096 | 171 (210) | 472 | 12,659 | 0.0217 | 1.8 | 99 | 12.4 | 27x |
| 8192 | 16384 | 166 (209) | 468 | 18,157 | 0.0606 | 6.7 | 142 | 4.4 | 39x |
| 8192 | 65536 | 174 (214) | 381 | 19,046 | 0.2309 | 32.6 | 149 | 1.2 | 50x |

The readout dominates at small k. For n=m=1024, the three readouts compared:

| k bits | readout | SME Gbit-op/CPU-s |
|---|---|---|
| 1024 | `MOVA` + `LSR` + `ST1` per row | 2,309 |
| 1024 | `ST1W` ZA slice, then a NEON halving pass | 4,549 |
| 1024 | SME2 `MOVA` x4 + `URSHL` x4 + `ST1` | 8,030 |
| 4096 | same three | 6,929 / 11,510 / 16,006 |
| 65536 | same three | 18,441 / 19,236 / 20,275 |

- At k ≥ 16 Kbit, SME runs at 80-95 % of the bare BMOPA issue rate at that run's clock, 40-50x
  NEON. The operand loads (140-158 GB/s) come from L1/L2 at the SME load-path limit seen in C2 and
  C7.
- Packing (plain scalar code, not optimised) is O((n+m)·k) against the product's O(n·m·k): 14 % of
  the product time at n = 8192, k = 64 Kbit, but as much as the product itself at n = 1024,
  k = 1 Kbit. It is paid once per matrix and could be amortised by storing bit rows panel-packed.
- At k = 1 Kbit, only 32 words of k are accumulated per 32 × 32 output block, so the run is bound
  by the readout and the output writes (24-34 GB/s of C). For n = 8192 that means writing 256 MB
  of u32 results to DRAM. Small-k products should emit narrower outputs (u16), or consume C
  in-register (a threshold, a top-k, a count) instead of storing it.
- Surprise: CSSC scalar `CNT` (one GPR popcount per instruction) is ~20 % slower than the
  `fmov` + NEON `cnt.8b` + `addv` sequence clang emits without `+cssc`.
- Scalar and NEON throughput do not depend on n or k (compute-bound at ~0.2 and ~0.47
  Tbit-op/s).

## C5. Sorted u16 array intersection count (Roaring array containers)

The reference is MLDB's `intersectionCountOptimized`:

- PCMPESTRM compares 8 × 8 u16 values all-pairs, and the match mask is popcounted.
- Each step advances `it1 += 8*(last1 <= last2)` and `it2 += 8*(last2 <= last1)`.

NEON equivalents:

- **rot:** `vceqq_u16` of the A block against the 8 rotations of the B block (`vextq_u16`). The
  masks are ORed, and match lanes are counted by subtracting the 0xFFFF lanes into an accumulator.
- **bcast:** the same, against the 8 lane broadcasts (`vdupq_laneq_u16`).
- **rot x2:** two independent intersections interleaved in one loop.
- **gallop:** exponential search, then binary search, of each small element in the large array.
- **NEON skip:** for each small element, skip 8-blocks of the large array by their last element,
  then compare with one `vceqq`.

All methods were verified against a bitmap reference on 32 pairs per configuration, in both
argument orders. Inputs are sorted unique u16 values:

- sparse: drawn from 0..65535;
- dense: drawn from 0..2·large, so about half the small array hits.

ns per element = CPU time / (|small| + |large|), minimum over 9 repetitions of 32 pairs. Load
30.8, clock estimate 2.6 GHz, P core.

| large | small | ratio | density | merge | NEON rot | NEON bcast | NEON rot x2 | gallop | NEON skip | best Melem/CPU-s |
|---|---|---|---|---|---|---|---|---|---|---|
| 16 | 16 | 1:1 | sparse | 2.60 | 0.32 | 0.32 | 0.25 | 1.04 | 0.64 | 3,973 |
| 16 | 16 | 1:1 | dense | 1.58 | 0.25 | 0.25 | 0.25 | 1.02 | 0.66 | 4,081 |
| 64 | 64 | 1:1 | sparse | 3.16 | 0.59 | 0.57 | 0.38 | 1.09 | 0.73 | 2,618 |
| 64 | 64 | 1:1 | dense | 2.38 | 0.49 | 0.50 | 0.36 | 1.01 | 0.74 | 2,818 |
| 64 | 6 | 1:10 | sparse | 2.68 | 2.58 | 2.58 | 2.64 | 0.54 | 0.18 | 5,537 |
| 64 | 6 | 1:10 | dense | 2.49 | 2.64 | 2.65 | 2.64 | 0.56 | 0.19 | 5,389 |
| 256 | 256 | 1:1 | sparse | 2.21 | 0.76 | 0.76 | 0.40 | 1.10 | 0.77 | 2,532 |
| 256 | 256 | 1:1 | dense | 2.29 | 0.78 | 0.77 | 0.40 | 1.08 | 0.77 | 2,534 |
| 256 | 25 | 1:10 | sparse | 2.85 | 0.70 | 0.72 | 0.41 | 0.83 | 0.20 | 4,921 |
| 256 | 25 | 1:10 | dense | 2.66 | 0.75 | 0.74 | 0.47 | 0.85 | 0.21 | 4,755 |
| 1024 | 1024 | 1:1 | sparse | 2.93 | 0.80 | 0.79 | 0.40 | 1.27 | 0.80 | 2,525 |
| 1024 | 1024 | 1:1 | dense | 2.24 | 0.78 | 0.78 | 0.39 | 1.57 | 0.81 | 2,541 |
| 1024 | 102 | 1:10 | sparse | 2.92 | 0.90 | 0.90 | 0.54 | 0.92 | 0.21 | 4,678 |
| 1024 | 102 | 1:10 | dense | 2.79 | 0.89 | 0.89 | 0.54 | 0.94 | 0.22 | 4,544 |
| 1024 | 10 | 1:100 | sparse | 2.68 | 1.16 | 1.16 | 0.91 | 0.19 | 0.074 | 13,578 |
| 1024 | 10 | 1:100 | dense | 2.70 | 1.07 | 1.07 | 0.80 | 0.19 | 0.073 | 13,678 |
| 4096 | 4096 | 1:1 | sparse | 2.85 | 0.77 | 0.77 | 0.39 | 3.56 | 0.96 | 2,577 |
| 4096 | 4096 | 1:1 | dense | 2.14 | 0.75 | 0.75 | 0.38 | 2.72 | 0.97 | 2,635 |
| 4096 | 409 | 1:10 | sparse | 2.91 | 0.77 | 0.74 | 0.39 | 0.95 | 0.23 | 4,383 |
| 4096 | 409 | 1:10 | dense | 2.80 | 0.77 | 0.77 | 0.39 | 0.97 | 0.23 | 4,403 |
| 4096 | 40 | 1:100 | sparse | 2.86 | 0.75 | 0.74 | 0.38 | 0.21 | 0.089 | 11,193 |
| 4096 | 40 | 1:100 | dense | 2.86 | 0.75 | 0.75 | 0.39 | 0.22 | 0.089 | 11,215 |

- The 8 × 8 NEON all-pairs step gives the PCMPESTRM algorithm a straight replacement. It is 3-4x
  faster than the branch-free scalar merge: ~0.75 ns/element, ~6 ns per step.
- The step is latency-bound: the next load address depends on comparing the two blocks' last
  elements. Interleaving two independent intersections halves the cost to ~0.39 ns/element
  (~2.6 G elements per CPU-second). **Batches of intersections should be interleaved**, not run
  one after another.
- Rotations and broadcasts cost the same.
- For skewed sizes, use the NEON block skip: 0.2 ns/element at 1:10 and 0.07-0.09 at 1:100.
  Scalar galloping is 2.5x slower than the block skip at 1:100, because sorted u16 containers are
  short enough that a linear block scan beats binary search.
- Density (match rate) barely matters, because the kernels are branch-free.
- These are cache-resident kernels at about 5 GB/s of input (0.39 ns per 2-byte element). They are
  nowhere near bandwidth-bound.
- SME offers no help here. `MATCH`/`HISTCNT` fault in streaming mode (probe from the brief), and
  the outer-product unit has no equality-compare outer product.

## C6. Bitmap containers (8 KB = 65,536 bits): AND + popcount

Methods:

- **Scalar:** CSSC `CNT`.
- **NEON:** `vld1q_u8_x4` + `vandq` + `vcntq` + add + `vpadalq`.
- **SME streaming SVE:** 512-bit `and`/`cnt`/`add` in streaming mode.
- **SME BMOPA diagonal:** the query's 16 words against the same 16 words of another bitmap. Only
  the diagonal of the outer product is useful, and it is extracted per bitmap.
- **SME BMOPA interleaved:** groups of 16 bitmaps are stored word-interleaved (`Xt[g][w][16]`). zn
  is query word w replicated across the 16 lanes and zm is word w of 16 bitmaps. Every tile row
  then holds 16 bitmaps' counts, and 4 tiles cover 64 bitmaps per pass.
  - The correction uses each bitmap's cardinality, which Roaring stores per container.
  - Building the interleaved layout costs 63 ms CPU for 32,768 bitmaps (setup).

Load 30.8, clock estimate 2.6 GHz, P core. All methods were verified against scalar counts.

Pair, both bitmaps in L1 (16 KB read per pair):

| method | ns/pair | GB/CPU-s |
|---|---|---|
| scalar (CSSC CNT) | 250.3 | 65.5 |
| NEON | 174.0 | 94.1 |
| SME streaming SVE | 244.4 | 67.0 |

One query against R others (bytes = R × 8 KB of others):

| R (MB) | scalar | NEON 1/pass | NEON 4/pass | SME streaming SVE | SME BMOPA diagonal | SME BMOPA interleaved |
|---|---|---|---|---|---|---|
| 128 (1 MB, L2) | 32.7 GB/s | 46.8 | 54.0 | 46.9 | 47.1 | **140.0** |
| 1,000 (8 MB) | 32.4 | 45.8 | 43.4 | 45.3 | 45.5 | **111.9** |
| 32,768 (268 MB, DRAM) | 32.1 | 45.1 | 27.8 | 41.6 | 38.3 | **80.1** |

Per bitmap, the interleaved BMOPA kernel takes 58.5 / 73.2 / 102.3 ns at the three sizes; NEON
(1 per pass) takes 175 / 179 / 182 ns.

- One core running NEON is **compute-bound** at ~45-54 GB/s at every size. AND, CNT and the
  accumulate cost about 3 instructions per 16 bytes. That figure sits below NEON's L2 read rate
  (66-81 GB/s), and at DRAM sizes about level with its single-thread DRAM rate (60 GB/s).
  Streaming-mode SVE is no faster: its vector ALU is slow, matching published notes.
- The diagonal BMOPA uses only 1/16 of the unit, and per-bitmap diagonal extraction (64 ZA row
  stores re-read by scalar loads per 4 bitmaps) cancels the gain. That cause is likely but was
  not isolated.
- **The interleaved-layout BMOPA runs at the SME load-path limit:**
  - 140 GB/s from L2, 2.5-3x NEON, at 1/16 outer-product utilisation. One BMOPA per 64-byte load
    at 1 per cycle is exactly the SME load bandwidth.
  - 80 GB/s from DRAM, against single-thread SME DRAM reads of ~100 GB/s.
  - The cost is the layout: bitmaps must be stored or transposed in 16-container groups.
- The 8 MB case runs between the L2 and DRAM figures. On a loaded machine, the P cluster's 16 MB
  L2 is shared with other tenants.
- Odd result: NEON with 4 bitmaps per pass falls to 28 GB/s on DRAM (5 concurrent streams per
  core), while 1 per pass holds 45 GB/s.

## C7. Memory bandwidth calibration

Read-and-reduce kernels. The cache-sized cases re-read a distinct region; the DRAM case streams a
512 MB array. GB per CPU-second, minimum (median). Load 30.8, clock estimate 2.6 GHz, P core.

| kernel | 64 KB (L1) | 1 MB (L2) | 4 MB (L2) | 512 MB (DRAM) |
|---|---|---|---|---|
| scalar, 4 add chains (ALU-bound) | 42.6 | 41.4 | 41.3 | 41.0 (40.7) |
| NEON `ld1 x4`, 8 accumulators | 107.9 | 81.5 | 66.4 | 59.8 (58.5) |
| SME streaming `ld1d x4` + EOR | 132.9 | 149.5 | 139.9 | 101.1 (92.9) |
| SME streaming `ld1w x4` → BMOPA | 123.2 | 159.8 | 154.1 | 106.4 (104.5) |

All threads (load 37.6). Each thread either streams its own slice of the 512 MB array in 2 MB
chunks, or re-reads its own 1 MB buffer. Per-thread GB/CPU-s by threads on the same cluster
(1 / 2 / 3 / 4 / 5 / 6), from the 12-thread runs:

| kernel, level | P core | M core |
|---|---|---|
| NEON, DRAM | 48.6 / 46.6 / 35.8 / 26.1 / 22.0 / - | 57.9 / 39.5 / 27.3 / 22.0 / 20.2 / 18.2 |
| NEON, L2 | 61.1 / 60.9 / 61.2 / 61.7 / 63.7 / 64.7 | 68.0 / 67.7 / 67.8 / 67.5 / 68.0 / 68.4 |
| SME → BMOPA, DRAM | 84.7 / 37.9 / 23.9 / 18.3 / 14.8 / 11.9 | 46.2 / 23.5 / 15.7 / 11.8 / 9.5 / 7.9 |
| SME → BMOPA, L2 | 132.4 / 70.9 / 49.6 / 35.9 / 27.2 / 19.5 | 50.2 / 26.1 / 17.4 / 13.5 / 10.8 / 9.8 |

Across all concurrent threads, NEON DRAM reads fall to ~26-30 GB/s per thread with 6 threads
on-CPU together and ~18 GB/s with 12, i.e. ≈ 180-220 GB/s aggregate. Other tenants share that
bandwidth; no 18-concurrent sample was clean.

- **L1:** NEON tops out at ~108 GB/s per core, about 40 B/cycle.
- **L2:**
  - NEON gets 66-81 GB/s per core and scales perfectly across cores.
  - SME's load path gets 150-160 GB/s, but it belongs to the cluster: P ≈ 120-150 GB/s in total,
    each M ≈ 50-60 GB/s in total.
- **DRAM:**
  - One core: NEON 60 GB/s, SME 100-106 GB/s.
  - Many cores: per-thread NEON rates fall to ~30 GB/s at 6 concurrent and ~18 GB/s at 12, so the
    aggregate stops growing at roughly 6-12 streaming threads (about 180-220 GB/s here).
  - SME DRAM streaming also time-shares per cluster.

## What this means for relational work

Where SME pays:

- **Dense bit-matrix products** with enough shared dimension: k ≥ 4-16 Kbit, i.e. 128-512 32-bit
  words. Examples are AND-popcount joins, set-overlap matrices, and closure steps over bit rows.
  - SME gives 18-20 Tbit-op/s on one P-cluster thread: 40-50x a NEON core, 100x scalar.
  - The whole chip adds about 2 × 7.5 Tbit-op/s from the M clusters.
  - The work is compute-bound on BMOPA issue, with operands from L1/L2. The SME load path (≈64 B
    per cycle) matches the 2x2 tile block exactly.
  - The ADDVA/ADDHA correction trick makes AND-popcount essentially free on top of the XNOR
    BMOPA.
- **One-vs-many bitmap AND-count** (semi-join or overlap of one set against many), if containers
  are stored 16-interleaved. SME's load path is the win here, not its arithmetic: 140 GB/s from L2
  and 80-100 GB/s from DRAM, against NEON's compute-bound 45-54 GB/s on one core.
- **Integer/float outer products** (int8 → 3.2 TOPS, fp32 → 1.5 TFLOPS per P cluster) for
  scoring or aggregation that is genuinely a matrix product.

Where SME does not pay:

- Short streaming blocks. Entry plus exit with ZA costs about 60 outer products; work fewer than
  ~1,000 MOPAs per block and you lose more than 10 %. FP blocks are worse still.
- Streaming-mode vector (non-outer-product) work. Streaming SVE and/cnt/add ran no faster than
  NEON.
- Diagonal-only uses of the outer product.
- Products with small k, which are bound by readout and output writes. They need narrower or
  consumed outputs.
- Sorted-array intersection: no streaming MATCH/HISTCNT, and no compare outer product.
- **More than one SME thread per cluster.** The unit is shared, so extra threads only burn CPU
  time waiting. Plan one SME worker per cluster (three on this chip, the P one about 3x the
  others) and give the remaining cores NEON or scalar work.

Where NEON is enough:

- Sorted u16 intersections. Use the 8x8 all-pairs block with two or more intersections
  interleaved (~0.39 ns/element, ~2.6 G elements/CPU-s at 1:1), and NEON block skip for skewed
  sizes (0.07-0.2 ns/element).
  - These are cache-resident and latency/compute-bound. Batching and interleaving matter more
    than the instruction set.
- Pairwise bitmap AND-count: 94 GB/s per core in L1.
- Bit-matrix products with small n·m or small k, where readout dominates and SME's edge shrinks to
  12-20x.

Where everything is bandwidth-bound:

- Any streaming pass over DRAM-resident containers at more than ~1-2 bytes of input per simple
  op. Examples are bitmap scans and AND-counts over many containers, and filters over columns.
  - One NEON core already needs ~45 GB/s for bitmap AND-count.
  - The chip's shared DRAM reads give ~18 GB/s per thread once 12 threads stream, about
    180-220 GB/s in total on this loaded machine.
  - So multi-core NEON saturates DRAM, and neither SME nor more cores help. The levers are
    layout and compression (Roaring array/run containers, smaller types), and reuse: blocking so
    that each byte fetched from DRAM feeds many ops.
- SME's compute is so high that dense bit products stay compute-bound only while operands live in
  L2. A 2x2 BMOPA block does 16 bit-ops per operand byte, so at 20 Tbit-op/s it needs ~150 GB/s.
  That is the cluster's whole SME load path, and more than its DRAM share. Hence the B-panel
  grouping in C4: it has to be blocked for L2.
