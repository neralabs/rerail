# Matrix-offload spikes

Probes and spikes for one question: how much of the logic engine's relational work (joins, semi-joins,
aggregations, set operations, closures, text tests) can run profitably on matrix hardware, restated as dense,
brute-force work: the Apple Neural Engine (ANE), the CPU's SME units, and the GPU (plain threads and its
matrix units). Memory bandwidth is treated as the limiting resource, so every result reports bytes moved as
well as time.

Nothing here is product code. The directories are measurement harnesses:

- `ane/`: Core ML programs built with coremltools (a dev-only Python environment) and run from Swift.
- `cpu/`: C kernels for NEON and SME (`clang -O2 -mcpu=apple-m4`; `-march=armv9.2-a` emits plain SVE,
  which faults on Apple silicon).
- `gpu/`: Metal kernels compiled at run time from Swift.
- `extract/`: the workload extractor over the data-layer corpus.

Results are in `RESULTS.md` (written as the spikes land). Timing rules: CPU time (user + sys) for CPU work,
device time for GPU work, wall time (minimum of many runs) plus host CPU for the ANE, whose execution is not
CPU time; the machine's load average is noted with every table.
