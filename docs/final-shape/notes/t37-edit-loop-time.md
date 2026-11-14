# t37. The edit loop: what makes it long, and what to do

## Intro

This note measures the edit, build and test loop and says what to change. It covers four questions:

- what makes the loop long and costly in CPU;
- Relax, a JIT for Rust;
- compiling on other machines (an M1 mac mini, an M1 Max and an M3 Max are idle on the LAN);
- the long-running tests.

The numbers are from 2026-11-14 on the M5 Max (18 cores). They were taken on the hub
(`final-shape-serverplan-quality` at 686ede10a). Every run was at `nice -n 19`. Other sessions
kept the load between 40 and 90, so wall times are inflated. CPU time (user + sys) is the figure
to compare.

## 1. Rebuilds are no longer the slow part

`AGENTS.md` recorded 135 s wall and 629 s CPU for a one-file edit (2026-11-10). The crate split
came after that. A one-file edit now rebuilds as follows (dev profile, `opt-level` 0 for the
workspace crates):

| Edit | `--bin rerail-engine` | `-p rerail-engine --lib` tests |
|---|---|---|
| a body in `typing.rs` (rerail-logic) | 19 s wall, 22 s CPU | 18 s, 26 s |
| a body in `facts/core.rs` (14 crates above it rebuild) | — | 31 s, 52 s |
| a new `#[test]` in `tape/tests.rs` | — | 15 s, 23 s |
| cold, with dependencies from sccache | 206 s | 186 s, 394 s |

Two avoidable costs show up in every test rebuild:

- **The engine is its own dev-dependency** (`rerail-engine = { path = ".", features = ["test-hooks"] }`).
  So every test build also compiles the non-test engine library and links its cdylib (365 MB in
  dev, 170 MB at O1). That costs 2.3 s of each O0 rebuild and 14 s of each O1 rebuild. The
  dependency is there only to turn the sub-crates' `test-hooks` on, and the dev-dependencies can
  name those features directly.
- **Any edit to a source file changes `RERAIL_ENGINE_BUILD_ID`**, test files included
  (`native/build.rs` hashes all of `src`). The CLI's default store is keyed by that id, so the
  first run after any rebuild is a cold run.

The test binary is 613 MB.

## 2. The test suite is the slow part

One run of the engine's library tests took 762 s wall and 2,682 s CPU (45 CPU-minutes):

| Per-test time | Tests | Their total time |
|---|---|---|
| under 1 s | 2,196 | 146 s |
| 1–5 s | 183 | 434 s |
| 5–20 s | 140 | 1,445 s |
| 20–60 s | 69 | 2,394 s |
| 60 s or more | 50 | 9,210 s |

The 119 tests of 20 s or more hold 85% of the time. They are corpus-scale runs inside the unit
suite. By module:

| Module | Tests | Their total time |
|---|---|---|
| `logic::plan::corpus` (session, reuse, phases) | 77 | 7,182 s |
| `logic::rr::tests` | 130 | 1,073 s |
| `logic::convert::tests` | 16 | 834 s |
| `logic::program_image` | 16 | 484 s |

Some of these tests are whole replays: a sample of the recorded replay on 8 threads of its own,
and the reference graph over every fixture event, one after another.

Per-test times count waiting as well as work. Each module figure is the sum of its tests' times,
so it can exceed the wall time of the run.

Three causes:

1. **A queue.** 32 corpus and session tests take `SESSION_TESTS`. The session is a process
   global (`static SESSION` in `corpus/session.rs`), so these tests run one at a time, each
   holding a test thread. Alone, one of them takes 9.3 s wall and 2.7 s CPU. Queued, their
   reported times reach 350 s. This chain is about half of the suite's wall time.
2. **Unoptimized engine code.** The dependencies run at O3, but the engine's crates run at O0. At
   O1 (`[profile.test-o1]`), with the same tests on the same binary sources:

   | Module | O0 CPU | O1 CPU | O0 wall | O1 wall |
   |---|---|---|---|---|
   | `logic::convert::tests` | 425 s | 111 s | 357 s | 100 s |
   | `logic::program_image::tests` | 298 s | 88 s | 97 s | 38 s |

   An O1 rebuild costs about 45 s more CPU than an O0 one: 43–68 s wall and 67–117 s CPU. An O1
   cold build of the test target took 1,900 s CPU.
3. **One process for every test.** A stack overflow in
   `compile::resolve::tests::syntax_detection_reads_a_deeply_nested_top_level` (a JS expression
   200,000 deep) aborts the whole binary 32 s in, so nothing after it reports.

Failures seen in this worktree:

- the stack overflow above, which is real;
- `language::adapter::kit::shared_walks_read_the_kinds_they_named` and
  `slice::tests::the_backends_find_the_same_keys_on_real_files`, not yet classified;
- the rest need the pack test data, which a fresh worktree lacks (`npm run fixtures:pull`).

## 3. Relax

Relax is makepad/relax, first shown in public on 2026-11-12. It has its own front end, type and
borrow checker, IR and AArch64 backend, and patches edited function bodies into a running process.
It does not use rustc. Its build scripts run confined, and they may pass no `rustc-link-arg`.
C code is built only with `--allow-c`, through cargo.

Tried on the leaf crates:

| Crate | Result |
|---|---|
| rerail-tape-format | `check` passes in 0.1 s. `test` passes once `--allow-c` is given, because `sha2-asm` compiles C. |
| rerail-simd8 | `test` passes, and its NEON backend runs. |
| rerail-rr-syntax | `check` passes on the second try; the first spent 150 s building the native dependencies. `test` passes 11 tests but was still running at 150 s. |

A one-line edit in rr-syntax compared with incremental cargo:

| | Wall | CPU |
|---|---|---|
| `relax check` | 13.5 s | 27 s |
| `cargo check -p rerail-rr-syntax` | 2.9 s | 0.8 s |

Its cache reached 6.4 GB for these two crates. It needs a working rustc for `--allow-c`, and it
cached a failed probe of the broken Homebrew rustc until `relax clean`.

The engine's crates need several things Relax does not document: bundled SQLite, tree-sitter's C,
simdjson's C++, jemalloc, Objective-C and Metal, Cranelift's JIT, and napi.

**Verdict: not for this workspace now.** On the code it can compile, it is slower than incremental
cargo. Look again when it builds C natively and is faster than `cargo check` on large crates.

## 4. Building on other machines

| Option | What it can do here |
|---|---|
| sccache-dist | Build servers must be Linux or FreeBSD. The Macs cannot serve. |
| icecream, distcc | No Rust support. |
| Bazel or BuildBuddy remote execution | A rewrite of the build into rules_rust, with no incremental builds. |
| sccache in general | Never caches incremental crates, nor anything that links. Only dependencies can be shared. |

What the idle machines can do:

- **A shared sccache backend.** Run MinIO (S3) or Redis on the mac mini. It helps fresh
  worktrees, dependency bumps and toolchain changes. Every machine and session must then use one
  rustc (see §5).
- **Remote test runs.** This saves the most CPU here. Full suites and gates (45 CPU-minutes each)
  run on the M3 Max or M1 Max over ssh, from a pushed branch, each machine keeping its own warm
  `target/`. The M5 keeps only edit, rebuild and targeted tests. Metal tests that need M3 or later
  GPU features stay off the M1s.

## 5. The environment

- **The toolchain.** Homebrew's rustc 1.97.1 aborts since the LLVM 23 upgrade, and runs only with
  `DYLD_LIBRARY_PATH` set to LLVM 22. `rust-toolchain.toml` pins 1.95.0, but Homebrew's cargo
  comes first on PATH, so the pin is ignored. Sessions mix Homebrew and rustup 1.97.1. Their
  builds share no sccache entries, and switching rebuilds everything (17 min).
- **One `target/` per worktree.** There are 20 of them, at 60–174 GB each, and each builds the
  same crates. On 2026-11-14 the disk filled to ENOSPC during this work, which stopped every tool.

## What to do, in order of value

1. **Make the session per run or per thread, not a process global, and drop `SESSION_TESTS`.**
   This removes about half of the suite's wall time.
2. **Put the tests of 60 s or more (about 50) in a slow tier,** run at the gate. Either ignore
   them with a reason and run them with `--include-ignored`, or give them a test binary of their
   own. The everyday run is then the other 2,500 tests.
3. **Run the slow tier at O1** (`[profile.test-o1]`, or `--profile profiling`): 3–4 times less CPU.
4. **Use cargo-nextest.** It gives:
   - a process per test, so a crash costs one test, not the run;
   - per-test times;
   - test groups, so only the session tests are serialized;
   - a slow-test timeout.
5. **Drop the engine's self dev-dependency** and name the sub-crates' `test-hooks` directly.
6. **Fix the stack overflow.**
7. **One toolchain:** rustup only, so the pin holds. Then add a shared sccache backend on the mac
   mini, and run the gates on the idle Macs.
