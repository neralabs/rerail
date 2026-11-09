# CPU probes: SME/SME2 and NEON for relational kernels

These are C microbenchmarks for the CPU's matrix unit (SME/SME2: 512-bit streaming vectors, ZA
tiles) and for NEON. They cover bit-matrix products, set intersections, bitmap AND-counts and
integer/float outer products, and they treat memory bandwidth as a first-class measure. Results
and conclusions are in `RESULTS-cpu.md`.

Every probe:

- times on the thread CPU clock, never wall time;
- reports the minimum and median over repetitions;
- prints the load average and a clock estimate;
- checks each kernel against a scalar reference before timing it.

## Build

```sh
make                      # binaries in ./bin (gitignored)
make OUT=/some/dir        # or elsewhere
```

The flags are `clang -O2 -mcpu=apple-m4+cssc`.

- `-mcpu=apple-m4` turns on the SME/SME2 ACLE intrinsics (`<arm_sme.h>`) without turning on plain
  SVE.
- Do **not** use `-march=armv9.2-a`. It lets the compiler emit SVE instructions outside streaming
  mode (e.g. `cntd`), and those fault with SIGILL on Apple silicon.
- `+cssc` makes `__builtin_popcountll` a scalar `CNT`; C4 measures both popcount forms.

## Run

Each run stays well under the 2-minute cap. Single-thread probes take 1-6 s of CPU; the
thread-scaling runs take about 50 s and 20 s of wall time.

```sh
bin/c1_streaming          # C1 smstart/smstop cost, outer products per block to amortise it
bin/c2_bmopa              # C2 BMOPA semantics (verified), AND-popcount derivation, throughput
bin/c3_mopa single        # C3 per-type outer-product throughput, one thread
bin/c3_mopa mt            # C3 thread scaling 1..18 with per-cluster attribution (~50 s wall)
bin/c4_bitmatmul 0        # C4 bit-matrix AND-popcount, n=m=1024 (args 1,2,3: n=2048,4096,8192)
bin/c5_intersect          # C5 sorted u16 intersection count (Roaring array containers)
bin/c6_bitmap             # C6 8 KB bitmap AND+popcount: pair and one-vs-many (uses ~530 MB)
bin/c7_bandwidth single   # C7 read bandwidth L1/L2/DRAM, one thread (512 MB array)
bin/c7_bandwidth mt       # C7 all-thread bandwidth, DRAM and L2 (~20 s wall)
```

Memory stays under 1 GB. The peaks are C4 at n=8192, k=64 Kbit (~512 MB) and C6 (~530 MB).

## Files

| file | contents |
|---|---|
| `bench.h` | CPU-time helpers. `TIME_CPU` gives min/median and records the core of the fastest repetition; `TIME_CPU_CL` splits repetitions by P/M core. Also the clock estimate and `run_chunked`, the multi-thread runner that attributes each work chunk to the number of threads observed on-CPU at the same time and on the same cluster. |
| `c1_streaming.c` | streaming entry/exit, raw and compiler-generated; FMOPA/BMOPA blocks of K = 4..65536 |
| `c2_bmopa.c` | BMOPA on known inputs, predicates, BMOPS, AND-popcount via corrections, tiles 1/2/4, operand feed from L1/L2 |
| `c3_mopa.c` | FMOPA fp32/fp16/bf16/fp64, SMOPA/UMOPA int8, int16→32/64, BMOPA, NEON control; scaling |
| `c4_bitmatmul.c` | scalar / NEON 4x4 / SME BMOPA 2x2 bit-matrix product, panel packing, ADDVA/ADDHA corrections, three readouts |
| `c5_intersect.c` | branch-free merge, NEON 8x8 all-pairs (rotations, broadcasts, two pairs interleaved), galloping, NEON block skip |
| `c6_bitmap.c` | pair and one-vs-many AND-count: scalar, NEON, streaming SVE, BMOPA diagonal, BMOPA over a 16-bitmap interleaved layout |
| `c7_bandwidth.c` | scalar / NEON / streaming-SVE / streaming-load→BMOPA read-and-reduce, single thread and all threads |

## Topology notes (M5 Max)

- Logical CPUs 0-5 and 6-11 are the two 6-core M ("Performance", `hw.perflevel1`) clusters; 12-17
  is the P ("Super", `hw.perflevel0`) cluster. This comes from IORegistry `cluster-type`.
- The running CPU number is read from the low bits of `TPIDR_EL0`.
- Each cluster has **one** SME unit, shared by its six cores (C3).
