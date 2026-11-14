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
borrow checker, IR, AArch64 backend and std, and does not use rustc. It runs a crate from source
and patches edited function bodies into the running process. C is built only with `--allow-c`,
through cargo. It runs on the host only, so the wasm build stays on cargo.

### What it took to run the engine

The whole engine runs under Relax: `relax check`, `relax test` and `relax run` of the CLI. The
CLI's outputs on the smoke corpus are the same bytes as the cargo build's. Getting there took four
changes, about 30 lines in all:

| Blocker | Whose | Fix | Size |
|---|---|---|---|
| E0252 on `logic::rr`'s `pub use print::print` | Relax: a path import binds another crate's `pub(in …)` fn that rustc skips as invisible | import the module alone, `rr::print::{self}` (b88a6d272) | 2 lines |
| Every session test hung | **the engine's**: the tests' store template was made from inside a run, and its filling thread's runs queued behind a test waiting to `set` the switches | the template is made before a test takes the switches (b038664e6) | 25 lines |
| `relax run` of the CLI: E0425 `vdup_lane_u32` (sha1dc), E0277 `AsRef<[T]>` for `vec::IntoIter` and `slice::Iter` (roaring), `core::array::IntoIter::new` (dary_heap) | Relax's std lacks them | added to Relax's std (below) | 23 lines |
| `--unattended` | none: it is an option of `run`, so it goes before the package | — | — |

The std additions are in Relax's source tree, not in this repository (`tmp/relax-src`, uncommitted):
an `AsRef<[T]>` impl on each of the two iterators, the deprecated `IntoIter::new` of an array, and
`vdup_lane_u32` as `uint32x2_t([a.0[N as usize]; 2])`. Relax reads its std from source, so they
need no rebuild of Relax.

The hang was a bug of ours that cargo's fast code hid: under cargo the window is milliseconds wide.
It may be the cause of 0204's phase 0 race too.

The bundled SQLite, tree-sitter's C, simdjson's C++, jemalloc, Objective-C and Metal, Cranelift's
JIT and napi all worked under `--allow-c`.

### Timings

All on the engine, at a load of 60–90. The cargo figures are incremental O0 builds with sccache.

| Loop | Relax | | cargo | |
|---|---|---|---|---|
| | wall | CPU | wall | CPU |
| `check` of the engine, cold | 206 s | 418 s | | |
| `check` after a one-line edit in logic | 57 s | 49 s | 14 s | 9 s |
| one session test, first run (JIT cold) | 94 s | 92 s | | |
| the same test, warm (cache filled) | 14–23 s (the test: 1.8 s) | 20–30 s | 8.8 s run (the test: 3.7 s) | 3 s |
| that test after a body edit in `corpus.rs`, plain `relax test` | 65 s | 119 s | 48 s (build and run) | 38 s |
| **that test after a body edit, `relax test --live`** | **2–9 s** | **about 6 s** | 48 s | 38 s |
| the same, after adding a function (a non-body edit) | 8 s | | | |
| the CLI on the smoke corpus, warm | 7.8 s | 12 s | 10–32 s run, after a 54–116 s build | |
| the CLI after a body edit, plain `relax run` | 121 s | 98 s | 64 s (build and run) | 42 s |
| the 15 session tests, first run after an edit to `switches` | 147 s | 247 s | 93 s (build and run, 17 tests) | 111 s |

In live mode the patch goes in within 0.1–2.5 s. A test that entered the changed function reruns
(`selected=affected`); a non-body edit reruns all the selected tests. A deliberate panic showed in
the next verdict, 1 s after the save, and undoing it passed 2 s later.

### What it costs

- **Memory:** the live test process held 6.2 GB.
- **Disk:** `target/relax` reached 13 GB for the engine, on top of `target/`.
- **Cold starts:** a first run compiles every function lazily and is slow (86 s for a test that
  runs in 3.7 s under cargo). The cache makes the next run faster than cargo's code (1.8 s).
- **Plain `check`, `test` and `run` after an edit are slower than cargo**, in wall and in CPU.
  They re-check whole crates (6,223 bodies for a one-line edit in logic).
- **It is experimental.** Its std is its own. It shares nothing with cargo's `target/`, and one
  std gap stops a run. Its gaps list is `libs/relax/KNOWN_GAPS.md`.

### Verdict

**Use `relax test --live` for the inner loop of one area's tests. Keep cargo for everything else.**
A body edit reaches a verdict in 2–9 s and about 6 s of CPU, against 48 s and 38 s for a cargo
rebuild and run: 5–10 times shorter and about 6 times less CPU. Plain `check`, `test` and `run`
lose to cargo after an edit, and builds for wasm, release, the gate and CI stay on cargo.

What adoption needs:

1. Send the std additions and the visibility bug upstream, or keep them as a patch on a pinned
   Relax commit.
2. A script, `scripts/dev/live-test.sh <filter>`, that starts `relax test --live rerail-engine
   --lib -- <filter>` at `nice -n 19`, with `RUSTC_WRAPPER=` and `RELAX_ALLOW_C=1`. It prints each
   `testverdict`, and the session stops it when the work is done.
3. A check, before a commit, that the same tests pass under cargo. Relax's std is not rustc's.
4. A disk budget: `target/relax` is 13 GB per worktree. `relax clean` when a worktree's work ends.

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

- **The toolchain (fixed 2026-11-14).** Homebrew's rustc 1.97.1 aborts since the LLVM 23 upgrade,
  and Homebrew's cargo came first on PATH, so `rust-toolchain.toml`'s pin (1.95.0) was ignored.
  The shells' profiles also sourced a rustup env file in another project's scratch directory. Now
  every new shell puts rustup's `~/.cargo/bin` first. A worktree then builds with its pin, 1.95.0,
  which has `wasm32-wasip1`. The browser build checks and builds again (39a3a3cde; 161 s, after
  `source native/wasm/env.sh`, which still uses Homebrew's clang and lld for the C). Shells started
  before the fix keep the old PATH, and each worktree rebuilds once on 1.95.0.
- **A newer Rust gains little for the edit loop.** Latest stable is 1.100.0 (2026-11-12); the pin
  is 1.95.0. The engine builds on 1.99.1 with no errors. Measured on one worktree, each toolchain
  with a target dir of its own, at load 40–80:

  | Build of the CLI | 1.95.0 | 1.99.1 |
  |---|---|---|
  | cold | 244 s wall, 461 s CPU | 189 s wall, 377 s CPU |
  | after a one-line edit in logic (three samples, CPU) | 29, 92, 23 s | 36, 84, 20 s |

  A cold build costs about 18% less CPU on 1.99 (one sample). Rebuilds after an edit, the loop
  that matters, are the same within the noise. An upgrade costs every worktree a cold rebuild
  (about 460 CPU-seconds each, 20 worktrees) and a change of CI's pin. 1.100 also stabilises the
  never type, which changes inference fallback, so it needs a check of its own. So upgrade for
  the language or for cold builds, all at once at a hub step, not for the edit loop.
- **One `target/` per worktree.** There are 20 of them, at 60–174 GB each, and each builds the
  same crates. On 2026-11-14 the disk filled to ENOSPC during this work, which stopped every tool.

## What to do, in order of value

Done on this branch:

- **The session is its caller's, not the process's** (decision 0204, 10a1fdef3). `SESSION_TESTS`
  is gone. The 129 corpus and typing tests run in parallel: 82–123 s wall, against the serial chain
  that was about half of the suite.
- **The engine's self dev-dependency is gone** (197f2de81). The test build no longer links the
  365 MB cdylib.
- **One toolchain** (§5): rustup first, the pin holds, and wasm builds.
- **The tests' template deadlock is fixed** (b038664e6, §3).
- **The stack overflow is fixed** (9bea7b866): the lean JS parser's destructuring patterns now
  count their depth, as its expressions do, so a deep pattern is refused, not recursed into.

Still to do:

1. **Put the tests of 60 s or more (about 50) in a slow tier,** run at the gate. Either ignore
   them with a reason and run them with `--include-ignored`, or give them a test binary of their
   own. The everyday run is then the other 2,500 tests.
2. **Run the slow tier at O1** (`[profile.test-o1]`, or `--profile profiling`): 3–4 times less CPU.
3. **`relax test --live` for the inner loop** (§3), with cargo checking before each commit.
4. **Use cargo-nextest.** It gives:
   - a process per test, so a crash costs one test, not the run;
   - per-test times;
   - test groups;
   - a slow-test timeout.
5. **A shared sccache backend on the mac mini, and the gates on the idle Macs** (§4).

