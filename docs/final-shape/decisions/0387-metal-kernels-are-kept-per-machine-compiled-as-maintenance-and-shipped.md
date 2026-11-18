# 0387. Metal kernels are kept per machine, compiled as maintenance, and shipped for the interpreters

Status: built (2026-11-18). Builds on 0012 (background compilation within a process), 0190 (upkeep), 0202 (the
record of forms runs went without; this builds its O162, O164 and O165) and 0386. Departs from 0202 §3's
assumption that macOS's shader cache covers repeated compiles: on the M3 it missed, and a run waited 26–33 s.

## Intro

A Metal pipeline costs two compiles:
- the **front end**, from MSL source to AIR, once per source;
- the **back end**, from AIR to the GPU's code, once per kernel and device.

Before this decision the engine kept compiled pipelines only in memory, so every process compiled again. It relied
on macOS's shader cache, a per-user cache that is evicted, to make repeats cheap. When that cache missed, a run's
critical path waited on the compile. The 300-file Metal run waited 26 s in two compiles, and its AST matcher wave
waited 16.5 s on 1.2 ms of GPU work (Q53).

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

## Measured

(Filled from the M3 runs below.)
