# Build-time baseline at 91c2df886 (before the crate split)

Measured at 91c2df8863 on a dedicated Linux build host (c7i.8xlarge, 32 vCPU, 61 GiB, Ubuntu 24.04, Rust 1.95.0), 2026-11-10 18:57–19:29 UTC (supplementary runs to ~19:55 UTC).

## Conditions

- **Host load.** The host was idle before the run (load 0.00/32, 60 G free). A 5 s sampler logged loadavg and any `rustc`/`cargo` not run from this checkout: every non-zero sample (22) falls inside the two cold builds and is cargo compiling registry crates; no other job ran during any build. Peak 1-min loadavg 17.6. The load before each warm build is the decaying tail of the previous build, not another job.
- **sccache.** The host has sccache configured; every build here ran with it disabled (`RUSTC_WRAPPER=` empty), and the sccache server recorded 0 compile requests before and after.
- `cargo fetch --locked` ran first, so no build includes downloads. Cold = `cargo clean` of the whole workspace target, then the build.
- **Build kinds.**
  - **profiling**: `cd native && cargo build --profile profiling --lib --bins --timings` (package `rerail-engine` and its path deps; profile `profiling`: opt-level 3, incremental, 256 CGUs).
  - **CI test**: CI's 'Rust engine (tests, …)' jobs run `npm run test:native` = `cargo test --manifest-path Cargo.toml --workspace` with job env `CARGO_PROFILE_DEV_DEBUG=line-tables-only CARGO_INCREMENTAL=0`; measured as `CARGO_PROFILE_DEV_DEBUG=line-tables-only CARGO_INCREMENTAL=0 cargo test --manifest-path Cargo.toml --workspace --no-run --timings` from the repo root (dev profile, deps at opt-level 3, **not incremental**, all workspace members and all test targets, not just `--lib`).
- Warm builds: `touch <file>`, rebuild; three times per file, median reported. CPU = `/usr/bin/time -v` user+sys.

## Results (4 builds × 2 kinds)

| Build | profiling wall (s) | profiling CPU (s) | CI test wall (s) | CI test CPU (s) |
|---|---:|---:|---:|---:|
| 1. cold (`cargo clean`) | 222.3 | 2496.6 | 216.2 | 1859.4 |
| 2. warm, touch `logic/rr/lower.rs` | 15.8 (17.9, 15.7, 15.8) | 17.4 (17.5, 17.3, 17.4) | 132.9 (135.6, 132.7, 132.9) | 343.2 (346.7, 343.2, 342.9) |
| 3. warm, touch `logic/plan.rs` | 16.5 (16.6, 16.5, 16.1) | 18.0 (18.2, 18.0, 17.8) | 136.0 (136.0, 132.8, 136.3) | 346.1 (346.1, 343.8, 347.1) |
| 4. warm, touch `cpython/syntax.rs` | 15.8 (15.8, 15.7, 15.9) | 17.3 (17.3, 17.2, 17.3) | 134.3 (133.3, 134.9, 134.3) | 345.4 (344.7, 345.4, 345.6) |

Warm cells: median (the three runs). Peak RSS: profiling cold 6.4 GB, profiling warm 2.9 GB; CI test cold/warm 9.6–9.7 GB.

### Reading the warm numbers

- **profiling, touch ≈ no codegen.** The profiling profile is incremental, and a `touch` changes no content, so every codegen unit is reused: `-Z time-passes` on the touched rebuild (below) shows `codegen_crate` 0.18 s and nothing for LLVM. The ~16 s is the incremental floor of the `rerail-engine` crate: dep-graph/query-cache serialisation (~5.4 s), crate metadata (~3.3 s), monomorphisation collection and partitioning (~3.1 s), expansion and resolution (~1.8 s), link (~1 s). All three touched files cost the same.
- **CI test, touch = whole crate.** With `CARGO_INCREMENTAL=0` a touch recompiles the whole `rerail-engine` crate twice in parallel (the lib test harness ~135 s, and the plain lib for the bin-test and example targets ~70 s); the touched file does not matter. Wall ≈ the lib test unit.
- **Supplementary: a real edit under profiling.** Because a touch hides codegen, each file was also edited (a `// bench` line prepended, shifting every span in that file) and rebuilt, then restored and rebuilt (`raw/edit-variant.sh`; file contents verified identical to the checkout afterwards):

| profiling, content edit | edit: wall / CPU (s) | restore: wall / CPU (s) | `rerail-engine` unit (edit) |
|---|---:|---:|---:|
| `logic/rr/lower.rs` | 86.2 / 764 | 87.6 / 779 | 85.1 s |
| `logic/plan.rs` | 90.3 / 839 | 90.0 / 838 | 89.1 s |
| `cpython/syntax.rs` | 86.5 / 749 | 86.1 / 754 | 85.3 s |

## `rerail-engine`'s own frontend vs codegen

`--timings` gives no frontend/codegen sections for `rerail-engine`: the lib is `cdylib`+`rlib`, emits no early rmeta, so cargo records only its duration (`rmeta`/codegen columns below are `–`). Durations from the timings: profiling cold `rerail-engine` lib **139.8 s** (of 222 s wall; it starts at 81.7 s and everything after it waits); CI test cold: lib test harness **137.6 s** ∥ plain lib **68.0 s**.

The split comes from a separate `RUSTC_BOOTSTRAP=1 cargo rustc … -- -Z time-passes` run on the same box afterwards (`raw/time-passes.sh`, `raw/time-passes/*.txt`). Frontend = total − codegen − link; codegen = `codegen_crate` + `finish_ongoing_codegen` (waiting on the LLVM workers).

| `rerail-engine` unit | total (s) | frontend (s) | of which type-check / borrowck / mono-collect / metadata | codegen (s) | link (s) | incr. dep-graph save (s) |
|---|---:|---:|---|---:|---:|---:|
| profiling lib, crate from scratch (incremental dir removed) | 138.0 | 62.5 | 16.8 / 17.2 / 18.3 / 22.4 | 74.6 | 0.9 | 1.1 |
| profiling lib, touch `rr/lower.rs` | 14.8 | 13.6 | 0.2 / 0.2 / 1.3 / 3.2 | 0.2 | 1.0 | 5.4 |
| profiling lib, touch `logic/plan.rs` | 15.2 | 14.0 | 0.2 / 0.2 / 1.5 / 3.5 | 0.2 | 1.0 | 5.5 |
| profiling lib, touch `cpython/syntax.rs` | 15.3 | 14.1 | 0.2 / 0.2 / 1.5 / 3.4 | 0.2 | 1.0 | 5.4 |
| CI test: lib test harness (`--profile test`, non-incremental) | 132.5 | 62.4 | 22.8 / 27.9 / 20.7 / – | 68.1 | 2.0 | – |

(The crate-from-scratch run also logs `LLVM_thinlto` 42.1 s inside the codegen wait.)

## 10 slowest units per build

Seconds. `rmeta` = end of the frontend section (when dependants could start), `codegen` = the codegen section; `–` where cargo records no sections (build-script runs, `cdylib`/test/bin units). Warm builds list every unit that did work (the rest were fresh); warm lists are from run 2 (the three runs rebuild the same units with durations within ~1 s).

### profiling — 1. cold (`cargo clean`) (`profiling-cold`: 222.3 s wall, 500 dirty units)

| unit | duration | rmeta | codegen |
|---|---:|---:|---:|
| `rerail-engine` | 139.8 | – | – |
| `libsqlite3-sys build-script (run)` | 60.5 | – | – |
| `cranelift-codegen` | 35.0 | 17.9 | 17.0 |
| `tikv-jemalloc-sys build-script (run)` | 29.2 | – | – |
| `syn` | 20.1 | 3.3 | 16.9 |
| `syn` | 19.9 | 3.3 | 16.6 |
| `regex-automata` | 18.1 | 3.2 | 14.9 |
| `zstd-sys build-script (run)` | 17.4 | – | – |
| `darling_core` | 15.5 | 1.5 | 14.0 |
| `cranelift-codegen-meta` | 13.8 | 1.6 | 12.2 |

### profiling — 2. warm, touch `logic/rr/lower.rs` (`profiling-rr-2`: 15.7 s wall, 3 dirty units)

| unit | duration | rmeta | codegen |
|---|---:|---:|---:|
| `rerail-engine` | 14.6 | – | – |
| `rerail-engine rerail-engine "bin"` | 0.7 | – | – |
| `rerail-engine build-script (run)` | 0.2 | – | – |

### profiling — 3. warm, touch `logic/plan.rs` (`profiling-plan-2`: 16.5 s wall, 3 dirty units)

| unit | duration | rmeta | codegen |
|---|---:|---:|---:|
| `rerail-engine` | 15.3 | – | – |
| `rerail-engine rerail-engine "bin"` | 0.7 | – | – |
| `rerail-engine build-script (run)` | 0.2 | – | – |

### profiling — 4. warm, touch `cpython/syntax.rs` (`profiling-cpython-2`: 15.7 s wall, 3 dirty units)

| unit | duration | rmeta | codegen |
|---|---:|---:|---:|
| `rerail-engine` | 14.6 | – | – |
| `rerail-engine rerail-engine "bin"` | 0.7 | – | – |
| `rerail-engine build-script (run)` | 0.2 | – | – |

### CI test — 1. cold (`cargo clean`) (`test-cold`: 216.2 s wall, 523 dirty units)

| unit | duration | rmeta | codegen |
|---|---:|---:|---:|
| `rerail-engine rerail_engine "lib" (test)` | 137.6 | – | – |
| `rerail-engine` | 68.0 | – | – |
| `libsqlite3-sys build-script (run)` | 60.7 | – | – |
| `cranelift-codegen` | 36.2 | 18.1 | 18.1 |
| `tikv-jemalloc-sys build-script (run)` | 29.9 | – | – |
| `syn` | 22.5 | 3.4 | 19.1 |
| `syn` | 22.2 | 3.3 | 18.9 |
| `zstd-sys build-script (run)` | 19.6 | – | – |
| `regex-automata` | 18.8 | 3.1 | 15.7 |
| `darling_core` | 16.7 | 1.5 | 15.2 |

### CI test — 2. warm, touch `logic/rr/lower.rs` (`test-rr-2`: 132.7 s wall, 5 dirty units)

| unit | duration | rmeta | codegen |
|---|---:|---:|---:|
| `rerail-engine rerail_engine "lib" (test)` | 132.2 | – | – |
| `rerail-engine` | 70.7 | – | – |
| `rerail-engine sql-executor-bench "example"` | 1.7 | – | – |
| `rerail-engine rerail-engine "bin" (test)` | 1.3 | – | – |
| `rerail-engine build-script (run)` | 0.2 | – | – |

### CI test — 3. warm, touch `logic/plan.rs` (`test-plan-2`: 132.8 s wall, 5 dirty units)

| unit | duration | rmeta | codegen |
|---|---:|---:|---:|
| `rerail-engine rerail_engine "lib" (test)` | 132.3 | – | – |
| `rerail-engine` | 70.5 | – | – |
| `rerail-engine sql-executor-bench "example"` | 1.7 | – | – |
| `rerail-engine rerail-engine "bin" (test)` | 1.3 | – | – |
| `rerail-engine build-script (run)` | 0.2 | – | – |

### CI test — 4. warm, touch `cpython/syntax.rs` (`test-cpython-2`: 134.9 s wall, 5 dirty units)

| unit | duration | rmeta | codegen |
|---|---:|---:|---:|
| `rerail-engine rerail_engine "lib" (test)` | 134.5 | – | – |
| `rerail-engine` | 69.2 | – | – |
| `rerail-engine sql-executor-bench "example"` | 1.7 | – | – |
| `rerail-engine rerail-engine "bin" (test)` | 1.3 | – | – |
| `rerail-engine build-script (run)` | 0.2 | – | – |

## CI wall times ('Rust engine' jobs)

From `gh run list -R neralabs/rerail --workflow CI` / `gh run view --json jobs`: job wall = completedAt − startedAt; the "Run" step is the `npm run …` step alone (cache restore ~0.5–1 min is separate).

### main

| run | head | created (UTC) | conclusion | fmt, clippy | tests, workspace and anchors | tests, the rest |
|---|---|---|---|---:|---:|---:|
| [42858821060](https://github.com/neralabs/rerail/actions/runs/42858821060) | 2b92045ac | 2026-11-10T16:51:21Z | success | 3.0 min (success; Run step 2.5) | 4.2 min (success; Run step 2.8) | 3.5 min (success; Run step 2.2) |
| [42855031906](https://github.com/neralabs/rerail/actions/runs/42855031906) | bd33c2a75 | 2026-11-10T16:29:25Z | success | 2.4 min (success; Run step 2.1) | 4.5 min (success; Run step 3.5) | 3.7 min (success; Run step 2.3) |
| [42850707932](https://github.com/neralabs/rerail/actions/runs/42850707932) | f5dc3404b | 2026-11-10T16:03:20Z | success | 3.0 min (success; Run step 2.6) | 4.2 min (success; Run step 2.8) | 4.1 min (success; Run step 3.1) |

### PR #508

| run | head | created (UTC) | conclusion | fmt, clippy | tests, workspace and anchors | tests, the rest |
|---|---|---|---|---:|---:|---:|
| [42865779308](https://github.com/neralabs/rerail/actions/runs/42865779308) | 5389cf332 | 2026-11-10T17:31:11Z | failure | 0.8 min (failure) | 17.0 min (failure; Run step 12.6) | 30.3 min (cancelled; Run step 28.6) |
| [42770106755](https://github.com/neralabs/rerail/actions/runs/42770106755) | 5a3935474 | 2026-11-10T06:40:37Z | failure | 0.7 min (failure) | 13.7 min (failure; Run step 9.5) | 30.3 min (cancelled; Run step 28.8) |
| [42704862037](https://github.com/neralabs/rerail/actions/runs/42704862037) | adb3e0d95 | 2026-11-09T21:41:24Z | failure | 0.8 min (failure) | 13.1 min (failure; Run step 11.1) | 30.3 min (cancelled; Run step 28.8) |

PR #508's last three runs all failed: 'fmt, clippy' fails in its Run step in under a minute (so it never reaches the clippy build); 'tests, workspace and anchors' passes its Run step (9.5–12.6 min) and then fails a later step ('Enforce broad producer execution shape' in the two latest, 'Test SQL and Rust reachability adapters' in 42704862037); 'tests, the rest' hits the 30 min job timeout inside its Run step (cancelled). So PR #508's wall times measure test runtime more than build time, and none is a complete job.

## Files

- `timings/<kind>-<build>[-<n>].html`: every cargo `--timings` report (profiling-*, test-*), plus `timings/edit/` for the content-edit runs.
- `raw/`: scripts (`bench.sh`, `time-passes.sh`, `edit-variant.sh`), `runs.tsv` (start/end epoch, loadavg before/after), `sampler.txt`, `env.txt`, `host.txt`, sccache stats, `/usr/bin/time -v` outputs (`*.time`), build logs, `time-passes/*.txt`.
