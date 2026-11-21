# 0395. Metal kernels are kept per machine, compiled as maintenance, and shipped for the interpreters

Status: built (2026-11-18). Numbered 0387 until the hub merge of 2026-11-18 (another worktree holds a 0387). Builds on 0012 (background compilation within a process), 0190 (upkeep), 0202 (the
record of forms runs went without; this builds its O162, O164 and O165) and 0386. Departs from 0202 §3's
assumption that macOS's shader cache covers repeated compiles: on the M3 it missed, and a run waited 26–33 s.

## Intro

A Metal pipeline costs two compiles:
- the **front end**, from MSL source to AIR, once per source;
- the **back end**, from AIR to the GPU's code, once per kernel and device.

Before this decision the engine kept compiled pipelines only in memory, so every process compiled again. It relied
on macOS's shader cache, a per-user cache that is evicted, to make repeats cheap. When that cache missed, a run's
critical path waited on the compile. The 300-file Metal run waited 26 s in two compiles, and its AST matcher wave
waited 16.5 s on 1.2 ms of GPU work (Q202).

The user asked for three things (2026-11-18):
1. a machine-wide store of compiled kernels, which can be pointed elsewhere, background compilation as maintenance,
   and a path that runs on the CPU while kernels are missing;
2. a foreground command that compiles every kernel the last run went without;
3. pre-compiled kernels shipped for the interpreted path, as the tier between optimized Metal and the CPU.

The second and third points are the agreed fallbacks this work may add: running elsewhere while a kernel is *not
compiled yet*. A kernel that *does not compile* stays fatal: the run stops and names the kernel.

## Decision

### 1. One path from source to pipeline: `rerail_accel::gpu::kernels`

Every compile in the engine goes through it:
- `rerail_metal::runtime::pipeline_named` (about 80 transducer sites);
- accel's joins, sorts, scans and tiles (`join::pipeline`, `join::compile`);
- the GPU lane's kernels.

Its in-memory maps replace the four the sites kept (`runtime`'s `PIPELINES` and `LIBRARIES`, accel's `LIBS`, the
lane's). `drop_pipelines`, the residency's drop, releases them with `kernels::release`.

### 2. The store

`RERAIL_METAL_KERNELS=<dir>|off`; the default is `~/Library/Caches/rerail/metal-kernels`, shared by every process
and worktree of the user.

```text
<store>/src/<library>.metal           a source a run wanted, or a fixed one it compiled (for maintenance, for ship)
<store>/fixed.list                    the fixed kernels runs compiled: "<library> <kernel> <constant>"
<store>/<device>/<pipeline>.archive   one pipeline's GPU code (MTLBinaryArchive); <device> is the GPU's name
<store>/<device>/wanted/<run>         the pipelines one run went without, a line each
<store>/<device>/<pipeline>.lock      held (flock) while a process compiles it
```

- **The back end runs once per machine.** A pipeline is compiled into a new archive, which is written to the store
  whole (a temporary file, then a rename). The pipeline is then made from that archive, so it is not compiled twice.
  A later process makes it from the archive (`FailOnBinaryArchiveMiss`).
- **A refused archive is a miss, not an error.** Metal refuses an archive built for another driver, for example
  after an OS update. The pipeline is then compiled again and its archive replaced.
- **The front end still runs per process**, from the source the caller holds, unless the source is shipped as AIR
  (§5).
- **Bounded.** The store's archives stay within `RERAIL_METAL_KERNELS_MB` (2 GB by default); the least recently
  used go first. A load touches its archive's modification time.

### 3. Who waits, and who runs another form: `pipeline` and `ready`

- `pipeline(source, name, constant)` is for a caller with no other form of the work. It compiles on the caller's
  thread unless the process or an archive has the pipeline.
- `ready(...)` is for a caller with another form. It returns the pipeline if it is at hand without a back-end
  compile. Otherwise it returns `None`, the caller runs its other form, and the mode (`RERAIL_METAL_COMPILE`)
  decides what happens to the compile:

| Mode | A kernel not compiled | Used by |
|---|---|---|
| `wait` | compiled on the caller's thread, and returned | the library's default: tests, embedders |
| `background` | compiled on a background thread of this process (0012's swap) | long-lived callers that ask for it |
| `defer` | recorded in the run's wanted list, for maintenance | every `rerail-engine` command but `kernels` |

The AST matcher wave is the first caller of `ready`:
- A wave asks for its generated pattern kernels (`LoweredWave::metal_ready`, whose sources are made once per wave).
- If they are not at hand, the wave's GPU lane has no price, so the executor places the wave on the cores. This is
  not an error fallback; the profile counts the wave under `msl-pipeline`.
- The wave's label kernels are the Metal interpreter's fixed kernels, so they are never pending.

The two existing users of `pipeline_ready`, block programs' generated forms and the span prefilter, keep their
behaviour through `ready`.

### 4. Maintenance and the foreground compile

- **After a run.** A `defer` run that went without kernels starts `rerail-engine kernels compile --all --idle` as
  it ends. That is a detached process in its own session, at the background priority, so it doesn't slow the
  foreground. `RERAIL_METAL_MAINTENANCE=off` turns it off; the benchmarks set it.
- **Upkeep.** Its job `metal-kernels` compiles wanted kernels, one per step, when the machine is idle, and prunes
  the store (0190).
- **The foreground command.** `rerail-engine kernels compile` compiles exactly the last run's wanted kernels, now
  and on this thread; `--all` compiles every run's. It reports each kernel's seconds and the front-end and back-end
  totals. `kernels list` shows the store, the shipped directory, the device and what is wanted. `kernels prune`
  bounds the store.

### 5. Shipped kernels for the interpreted path

The interpreters' kernels, and the relational kernels they launch, have fixed sources: the same for every rule set.
Their call sites use `pipeline_fixed`, which records them in the store's `fixed.list`:
- the label passes, the resident interpreter and the sweep;
- the transducer's batch interpreter;
- the block-program interpreter;
- accel's joins, sorts, scans, tiles and lane kernels.

`rerail-engine kernels ship <dir> --from <store>` builds the shipped directory from a reference run's manifest:
- this device's archive of each kernel, so the back end is skipped;
- with `--air`, each library's AIR from Xcode's offline compiler, so the front end is skipped too, on any Apple GPU.

Run on another device into the same directory, `ship` adds that device's archives. The engine reads
`RERAIL_SHIPPED_KERNELS=<dir>|off`, by default `rerail-kernels/` beside the engine. Its archives are tried before
the store's, and its AIR before a front-end compile.

The three tiers are:
1. a generated kernel, once maintenance has compiled it;
2. the interpreter's shipped kernels, always ready on a device they were shipped for;
3. the CPU, while a generated kernel is missing.

### 6. Every backend: fixed kernels compiled ahead (2026-11-21)

The device layer's store (`rerail_device::store`: CUDA's cubins, OpenCL's binaries, the CPU's shared objects) is
the same tier for the other backends. What reaches it ahead of a test or a run:
- **One list of the fixed kernels.** Each crate names its own as `rerail_gpu::kernels::Ahead` entries, each asking
  for its kernel as its use does (so the same key): `rerail_accel::gpu::kernels_ahead` (the UKL library, the joins'
  small path, sorts and gathers, the relational kernels), `rerail_metal::kernels_ahead` (the block interpreter at
  each width, the preprocessor's passes), the device's (`MatcherDevice::kernels_ahead`: the join stages, the chunk
  views, the set steps, the lattice, the nearest jumps, the interpreters and the label passes) and, with the tests'
  hooks, the tape format's tests' kernels. The engine's `kernels_ahead` (`tools.rs`) joins them.
- **`rerail-engine kernels compile`** compiles the list on every backend, `--threads` at a time (half the cores by
  default), then (Metal) the wanted kernels as before. `--within <s>` bounds it: no compile starts after `s`, and
  none is waited for past `s + 60`; the next call goes on from what the store kept. `kernels ship <dir>` copies the
  entries the list's compiles looked up (`store::recording`) into `<dir>/<backend>/`, by the store's keys, and the
  store reads a shipped form first (`RERAIL_SHIPPED_KERNELS`, else `rerail-kernels/` beside the engine).
- **The tests.** nextest's setup script `kernels` (`.config/nextest.toml`) runs `kernels compile --within 120`
  before the tests of the crates that launch kernels, on the run's device, with the tests' features.
- **A run.** The GPU lane's prewarm (`join::prewarm`) compiles `rerail_accel::gpu::kernels_ahead` on the pool as the
  lane is first asked for; the label passes keep their own warm-up (`label_passes::warm`).
- **What is left out.** A fixed source a device's compiler does not compile in bounded time is not on that device's
  list: the preprocessor's passes everywhere but the CPU (where they run: NVRTC grew past 90 GB resident on a DGX
  Spark, Intel's OpenCL was still compiling at 90 s), and the label sweep on CUDA (NVRTC not done at 500 s). These
  are compiler costs to fix in the sources, not settled.
- **OpenCL: one program a kernel.** A driver builds every kernel of a program. Intel's (an Arc 140V) took 436 s over
  the 21 join stages of one source, past the 240 s a test may take. On OpenCL each fixed kernel is now a program of
  its own (`UKL_ONE_KERNEL`, `__UKL_ONLY_<name>`: every other kernel of the source is only its body, a static
  function nothing calls): 157 s for all 21 at four at a time, 1.5–2 s for the kernels that read no view, and a run
  compiles only the kernels it launches. The seven that read views (`vread`) still take 50–100 s each: the driver
  inlines the view reads at every call despite `UKL_COLD`, and with `-cl-opt-disable` the same kernel builds in
  1.9 s. Still open.

## Measured

All runs on the M3 (t44, t45), on the 300-file, 317-rule shape with every thread. The machine was shared, with load
averages of 33–79, so wall times vary by a second or two between repeats.

`RERAIL_DIAG_MSL_NONCE` makes every source new to macOS's shader cache, the worst case for a first process on a
machine. The runs:
- **A:** an empty store; generated kernels are deferred, and fixed ones are compiled in the foreground.
- **B:** the store A filled, plus `kernels compile --all`.
- **C:** an empty store, with A's fixed kernels shipped (`kernels ship`).

| Run (cold) | CPU s | Wall s | Notes |
|---|---|---|---|
| t43 (0386, before this) | 52.5 | 20.0 | macOS's shader cache only |
| A: empty store | 55.1 | 32.9 | load 79 at its end; profiled, 57.6 / 15.3 |
| B: the store filled | 52.2 | 17.6 | every kernel from its archive (18 loads, 7.7 ms) |
| C: shipped fixed kernels, empty store | 47.0 | 13.8 | the wave on the cores until its kernels compile |
| t45, nonce, empty store | 52.3 | 15.1 | |
| t45, no nonce, empty store | 56.2 | 14.0 | |
| the CPU alone (t44) | 61.7 | 15.5 | |

- **What compiles cost.** `kernels compile --all` after A compiled what A went without, `tempo_batch` and one
  `clause_patterns`, in 0.33 s: 0.29 s front end, 0.02 s back end. Profiled A spent 100 ms in the front end and
  12 ms in the back end on its lane threads; profiled B spent 1.4 s in front ends, the pattern kernels' sources,
  which no AIR covers.
  - So the 26–33 s waits of t42 and t43 were not the compiler's own work. They came through macOS's shader cache,
    which several worktrees and test runs on the host share, while the host was busy. Making pipelines from our
    archives goes around that cache.
  - Shipping AIR (`--air`) would save front ends of 0.1–0.3 s for each fixed library, so the Metal Toolchain is not
    worth requiring for now.
- **Maintenance.** A `defer` run that wanted one kernel started the detached compile at once; 15 s later the kernel
  was archived and the wanted list removed.
- **Shipping.** `kernels ship` from A's store shipped 3 fixed libraries, 7 kernels, in 0.45 s, as this device's
  archives.
- **Findings** were 463 at cold and 464 after each edit in every run, as on the CPU.
