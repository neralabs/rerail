# t37. The edit loop: what makes it long, and what to do

## Intro

This note measures the edit, build and test loop and says what to change. It covers four questions:

- what makes the loop long and costly in CPU;
- Relax, a JIT for Rust;
- compiling on other machines;
- the long-running tests.

The numbers are from 2026-11-14 on the development machine (18 cores). They were taken on the hub
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

### With the farm (2026-11-15)

The same body edit in `logic` (`session.rs`, `start`), rebuilt with the global sccache every
worktree uses and with the farm (§4), each on a warm target:

| Rebuild after the edit | default: wall, CPU | farm: wall, CPU |
|---|---|---|
| the CLI, load 5–17 | 6.4–7.2 s, 17–18 s | 6.6–6.8 s, 19 s |
| the library's tests, interleaved, load 65–115 | 18.8, 22.2, 61.7 s; 41–48 s | 23.9, 24.6, 40.2 s; 43–46 s |
| the CLI, the first edit of a crate that was a cache hit | — | 32.7 s, 78 s |

In steady state the farm costs what the default does: a workspace crate being edited compiles
here, incrementally, either way. The farm's gain is the first build of a worktree (§4, 28 s from
a filled cache). Its one cost is the first edit of each crate that came from the cache: no
incremental state came with it, so that edit compiles the crate and those above it whole, once.
Before 33033b578 that first edit went to another Mac as a whole compile (20.5 s for `logic`), and
the next one compiled whole again here.

The farm's keys hold the target directory's path within the checkout: a worktree building into
`target/` shares with every other one; a `CARGO_TARGET_DIR` elsewhere shares only with the same.

Found on the way: since 197f2de81 (the self dev-dependency's removal), only `--lib` test builds
compiled. `--lib --tests` and `cargo test --workspace` (CI) also build the plain engine library,
where feature unification turned the variant `AstMatcher::TestUnknown` on and its match arms off.
The variant is now always there (e26cc1d5b).

### One feature set (2026-11-16)

The disk filled: the worktrees' target directories held 908 GB, 575 GB of it incremental state,
406 GB of that unused for 2 hours. One worktree held 44 incremental directories for
`rerail_logic` alone, 1–3 GB each. Each is a variant of the crate: a feature set, profile and
dependency graph some command built it with. The cleaner kept each for a day (now 2 hours).

The variants came mostly from feature unification. The engine's dev-dependencies turn on
`test-hooks` in 16 workspace crates, 16 more grammars in `ast-grep-language`, and rusqlite's
`array`. cargo applies dev-dependency features only to builds of test targets, so
`cargo build --bin rerail-engine` built every dependency crate without them and
`cargo nextest run --lib` built every one again with them: each edit compiled each crate above
it twice, and kept two sets of incremental state. `cargo test -p <crate>` (the crate's
`default`) and `--features metal-tests` add more.

The engine's `test-hooks` feature now turns on everything the dev-dependencies add, and a
development build of the CLI uses it (`AGENTS.md`). `cargo tree` then gives both builds the same
features for every crate but the engine itself and the test-only `rerail-parser-tests`.
`scripts/check/check_dev_features.py` (in the pre-commit hook) checks that the feature keeps
up with the dev-dependencies, target-specific ones included, and the farm's wrapper tells a CLI
build without it. Product and `profiling` builds are unchanged.

A crate's own tests made variants too. `cargo test -p rerail-logic` gave logic its `default`, which
the engine's builds did not (they take the crates without defaults), and it gave `rerail-metal` no
hooks: the hooks reached it only through the engine's own dev-dependencies. Two manifest rules fix
both, and the check enforces them: each crate's `default` turns on the `default` of the workspace
crates it takes without defaults, and each crate's `test-hooks` turns on the `test-hooks` of every
hooked workspace crate below it, its own test build turning on its own. `cargo tree` then gives
every workspace crate the same features in the CLI build, the engine's tests and each crate's own
tests (registry crates still differ in a crate's own tests, but they compile once into the cache,
with no incremental state).

What the rules miss, the farm's wrapper catches as it happens: it records the feature set of each
workspace crate's compile per target and profile, and a new one within 2 hours of another is told
in that build's output and at the checkout's builds for 3 hours, with the differing features, the
earlier command and what to do. A new dependency graph with the same features (after a Cargo.toml
change) is not told.

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
  `slice::tests::the_backends_find_the_same_keys_on_real_files`, classified below;
- the rest need the pack test data, which a fresh worktree lacks (`npm run fixtures:pull`).

### Two tiers, and the gate on another Mac (2026-11-15)

cargo-nextest runs each test in a process of its own, and reports each test's time. The stack
overflow then fails one test, not the run, and a process-wide session no longer queues the
session tests. `.config/nextest.toml` sets two tiers:

```
cargo nextest run --lib                   # the quick tier: all but the slow tests
cargo nextest run --lib --profile gate    # everything (a phase gate)
```

The slow tier is the tests that took 20 s or more each in a full run: 45 tests, named by family
in the config. A filter on the command line narrows a tier; `--ignore-default-filter` runs a slow
test by name.

`scripts/dev/test-remote` sends this checkout to a farm host through the mirror and runs a command
there (`scripts/dev/test-remote -- cargo nextest run --lib --profile gate`). The host builds in
a warm target directory of its own (one per checkout, under the farm's root there; §4, "What
broke"), so the suite spends the host's CPU, not this machine's. On an M3 Max at
`-j 16`:

| Run | Tests | Wall |
|---|---|---|
| The gate | 2,639 | 234 s |
| The quick tier | 2,594 | 87 s |

In the gate's run the tests took 3,460 test-seconds in all:

| Per-test time | Tests | Their total time |
|---|---|---|
| under 1 s | 2,319 | 252 s |
| 1–5 s | 216 | 469 s |
| 5–20 s | 61 | 587 s |
| 20–60 s | 30 | 960 s |
| 60 s or more | 13 | 1,191 s |

The longest test, `convert::tests::a_sample_of_the_replay_gives_the_recorded_findings` at 227 s,
bounds the gate's wall time. The next is `rr::tests::dropping_unreached_clauses_…_script_packs`
at 136 s.

The gate's 14 failures:

- **11 need the pack test data**, which neither machine has: the ruff, sqlfluff and eslint-plugin-vue
  locks were rewritten after the data moved to S3 (#177, #191), so git history cannot restore it,
  and a pull needs S3 access. These are the `python_shortcuts_change_no_pack_result_*` tests,
  `reproduces_sqlfluff_rf01_and_al05`, `vue::view::tests::corpus`, the three that read
  `pyflakes/F601.py`, and three more that read the pack's upstream files and found few or none:
  `the_backends_find_the_same_keys_on_real_files` ("1 files"), `the_gate_catches_a_widened_walk`
  and `spilled_walks_find_the_cpus_roots`.
- **1 needed `node`**, which the host's ssh PATH lacked (`sql_sessions_use_shared_explain_profiles…`);
  `test-remote` now puts Homebrew's bin on it, and the test passes there.
- **2 were real, and are fixed:**
  - `executors_are_shared`: the MSL compile thread (`metal/src/runtime.rs`) and the cost model's
    reader (`logic/costs.rs`) made their threads themselves; both are `exec::service` threads now.
    The build script's grammar threads (`build_grammars.rs`) are an exception: they run in cargo,
    not in the engine.
  - `shared_walks_read_the_kinds_they_named`: the brace-format tree names a kind `attribute` (a
    field's `.name`), and the format family's scope is refused. The walk kinds are now checked only
    for families with scopes.

On a host, two more things differ from this checkout. The store's git tests made repositories
under the host's own git config, and a host that signs its commits failed 10 of them; their git
now runs without the global and system config. And
`git_workspace::tests::discovers_current_repository_without_a_git_process` read this checkout's
`.git`, which the mirror does not send, so it failed on a host only; it is
`discovers_a_repository_from_inside_it_without_a_git_process` now, over a repository of its own
(2026-11-19). A command over ssh also starts
with 256 open files (a terminal has far more), and 17 of the engine's session, plan and executor
tests ran out of them in parallel; `test-remote` and `live-test-remote` now raise the limit to
`kern.maxfilesperproc` first.

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

The std additions are `scripts/dev/relax/std.patch`: an `AsRef<[T]>` impl on each of the two
iterators, the deprecated `IntoIter::new` of an array, and `vdup_lane_u32` as
`uint32x2_t([a.0[N as usize]; 2])`. `scripts/dev/relax/build` builds the pinned Relax with its patches into
`~/.local/share/rerail-relax/v/<commit>-<patches' hash>`, a tree never changed once built, and
installs its binary as `bin/relax`. The binary reads its std from the tree it was built from, so
a change to a patch makes a new version. The script does not switch versions while a Relax
session runs.

Sharing between worktrees (2026-11-15). Relax keys a run's stores by its worktree's absolute
paths, so another worktree's first run of the same code was cold: 288 s against 5–8 s warm. Upstream
(5b2d68c2, Target B steps 3–6) shares sealed crates' code (registry crates whose dependencies
have no OUT_DIR) in `~/.relax/code`, but only in its sandboxed `makepad` runs.
`scripts/dev/relax/rerail.patch` lets bare `relax` share too under `RELAX_SHARED_OPEN=1`; the
cache configuration hash keeps its entries apart from sandboxed ones. Measured on the session
test, at a load of 75–90:

| Run | Wall | CPU | Peak |
|---|---|---|---|
| worktree A, cold | 572 s | 785 s | 6.0 GB |
| worktree C (same code), cold, A's shared code | 385 s | 702 s | 6.0 GB |
| worktree A, warm | 27 s | 30 s | 3.7 GB |

The shared code is 50 MB. The rest of the store (front_run, collected_graph and the workspace's
own crates, 2.3 GB) is per worktree, so sharing saves about a tenth of a cold run. More waits on
Target B's path-free payloads upstream.

The patch also makes a stubbed build script (`--allow-c`'s cargo run) a two-line shell script.
It was a hard link of Relax's own binary, and cargo removes and links build scripts' names as
it rebuilds them. macOS's syspolicyd then could not read the file by the name it was scanning
("Unable (errno: 2) to read file") and killed the running Relax ("Terminating process due to
Malware rejection").

Sharing the workspace's crates (2026-11-15). The fork, its setup and the invariants are in `scripts/dev/relax/README.md`. Under `RELAX_SHARE=rerail` Relax mounts each worktree at
one path, so the keys of the workspace's own crates match across worktrees: a second worktree's
first run of the same code took 15 s wall, 27 s CPU. But two worktrees whose code differed undid
each other's stores, and each turn cost what a run after an edit did (60–75 s wall, about 200 s
CPU; 115–183 s at a load of 50–85). The cause was the native receipt. The engine's build script
compiles C, so the native build's key hashes the engine's files, and its `rustc-env` build ID
hashes all its sources. Relax gave every build script that reaches a native package (all reach
libc) the receipt's path in its environment, and the receipt names every native package's outputs
and the library's folder (the native key's). After any engine edit every script ran again in a new
OUT_DIR, and every crate whose closure reads one (24 stores, all above `unicode_names2`) went
stale. rerail.patch keys a script on `RELAX_NATIVE_RECEIPT_ID` instead: the outputs of the native
packages it reaches, their files when they are path packages, and the library's archives and
packages as sets (the link lists them in no fixed order; a first version hashed the list and
still missed). Taking turns, A then C (a one-line edit) at a load of 35–65:

| Run | Wall | CPU | Cache hits / misses |
|---|---|---|---|
| A, cold (new Relax build) | 87 s | 354 s | 0 / 61647 |
| C after A | 30 s | 113 s | 55924 / 1436 |
| A after C | 40 s | 114 s | 55946 / 1201 |
| C after A | 26 s | 97 s | 56016 / 1169 |

Before the fix the same turns took 115–183 s wall, 242–272 s CPU, with about 22000 hits. What
each turn still redoes is the engine's own crate (its build ID, and `RERAIL_FIXTURES_DIR`, an
absolute checkout path the build script sets) and the native cargo step.

Two runs at once (2026-11-15), A and C as above, both from the installed Relax under
`RELAX_SHARE=rerail`, at a load of 55–75. Both cold: 152 s and 156 s wall (471 s and 328 s CPU);
the second waited on the first's build-script claims (`waits for its claim`) and took their
outputs. Both again: 28 s and 70 s wall. The first try found that two native builds at once list a
package's host and target runs (libc) in either order, which gave the same outputs two identities;
the identity takes every list as a set now.

Bounded and remote (2026-11-15, later). `clean-target` removes Relax run dirs idle for 6 hours,
the build script keeps two installed versions, and the store cap is a fixed 8 GB
(`RELAX_CACHE_MAX=8192`). The engine's build script no longer writes the checkout's fixtures path,
so worktrees with the same code share the engine's store too: C after A 10 s, A again 6 s. Two
live sessions side by side (A and C) each patched only their own checkout's edits, 3–10 s after
the save. `live-test-remote` now runs as here (`RELAX_SHARE`, the cap) with its home under the
host's farm root, and sends the open-file limit (it did not, though §2 said so): on the M3 Max a
cold first verdict in 160 s, then 2–3 s per body edit. Details in `scripts/dev/relax/README.md`.

Quiet runs (2026-11-15, farm 9). Benchmarks among a dozen agents measured whatever else was
running. `scripts/dev/quiet-run` leases a machine (a host, or this one) for a command: fair turns
by the last hour's use, one request per checkout, 10 minutes at most a run (first 5 minutes, and
more only while nobody wanted the machine; set to a flat 10 the same day). While leased the farm sends it no compiles or test commands, and here every
compile waits. Work the farm does not control (tests, Relax) is reported by checkout, and that
checkout's builds are told it was unfair. Tested here, on the M1 Max, queued (two checkouts) and
stopped when wanted (since removed). Here the machine never got idle in these tests: 5–18 cores busy at the
start, mostly Spotlight, media analysis and other agents' tests. So the report, not the lease,
is what makes a local benchmark's number readable. A host is the place for a quiet one.

Hub code under Relax (2026-11-15). Relax rejected `ColMap::of_rel` (plan/table.rs) with E0310, which
rustc accepts. The reduction: an edition 2024 `impl Iterator` returned from `fn f(&self, _r: &str)`
(so it captures both lifetimes) that holds a closure built inside another closure, which captures
`self`. Relax gives each region slot of a closure type an identity, and a slot that is one of the
parent's own regions is mapped to that region when the nested body's requirements are applied,
so the nested body propagates nothing for it. The parent's instantiation of the closure type had
a fresh variable there that nothing else constrained when the slot was in the signature: the inner
closure's borrow reached the hidden type free, and between `'a` and `'b` (unrelated) member
constraints chose `'static`. rerail.patch ties those signature slots to the parent's regions
(borrowck/root_cx.rs `tie_inherited_slots`; upvar slots are left alone, since an or-pattern's binding
records only one of its regions, and tying it rejected `chunked.rs` `combine`). Relax's 272 borrowck
tests pass, the hub's session test passes under Relax, and the reduction is a test that fails
without the tie.

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

### On another Mac

The same live session, run on the M3 Max (idle, 16 cores) from the farm's live mirror of this
checkout, with the edit made here and its file pushed through the mirror (`scripts/build/farm`):

| Step | Time |
|---|---|
| start to the first verdict, cold (native dependencies, then the JIT) | about 140 s |
| a body edit in `session.rs`, saved here, to the M3's verdict | **1.7 s and 1.1 s** |
| a deliberate panic in the same function, to its failed verdict | 0.7 s |
| undoing it, to the passing verdict | 1.2 s |
| sending the edited file from here | 0.014 s |

This machine spends nothing but the push: the JIT, the 6–7 GB live process and the 7.2 GB
`target/relax` store are the M3's. It is also faster than the same session here (2–9 s at a load
of 60–90), because the M3 is idle.

`scripts/dev/live-test-remote` makes it one command:

```
scripts/dev/live-test-remote rerail-engine --lib -- <filter>    # the Relax of scripts/dev/relax/build
```

It sends the checkout and Relax to the first `"mirror"` host, starts the live session there, and
watches this checkout (FSEvents), sending each saved file; the host's verdicts print here, each
with the time since the save. Ctrl-C, a kill or a closed terminal stops the session on the host.
Measured with it: a restart with the host's store warm, 4–11 s to the first verdict; a panic
added to `start`, failed 1.6 s after the save; undone, passed 0.9 s after.

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

### The farm (`scripts/build/farm`, 2026-11-14)

Built instead of the options above: a `RUSTC_WRAPPER` with one sccache server for every worktree,
and two other Macs for its misses (an M3 Max with 16 cores and an M1 Max with 10, the second in use).
The README has how it works. Three parts:

- **One cache for every worktree.** A patched sccache (upstream main with PRs #3021 and #3022)
  hashes paths relative to the worktree and remaps them in the outputs. Decision 0224 removed the
  checkout's path from the engine's crates. The farm also makes proc macros path-free
  (`--remap-path-prefix`, a relative install name and debug map), since every crate above one
  hashes its dylib.
- **Misses on other Macs.** A library crate's compile goes to a host with free cores. Its sources,
  its dependencies' `.rmeta` files and its `OUT_DIR` are copied there (only what changed) and the
  outputs come back. Build scripts, proc macros and binaries stay here.
- **Edits stay incremental.** A workspace crate that missed in the last 2 hours compiles
  incrementally here, past the cache.

A cold build of the CLI (`--bin rerail-engine`, `-j 36`, `nice -n 19`, load 45–100 from other
sessions). CPU is this machine's: cargo's own (build scripts, proc macros) plus the farm server's,
which runs every other compile.

| Build | Wall | CPU here | Hits |
|---|---|---|---|
| empty cache, every compile here | 262 s | 1,273 s (270 + 1,003) | 0 of 378 |
| empty cache, misses on the other Macs | 316 s | 734 s (336 + 398) | 0 of 378; 354 compiles remote, 382 s there |
| a second worktree, cache filled by the first | 170 s | 402 s (293 + 110) | 376 of 378 |
| the same, after the C builds became path-free | 142 s | 366 s (273 + 93) | 378 of 378 |

- Sharing is the large saving: a new worktree, or one that switches branches, compiles almost
  nothing. The two misses were `rerail_lang` and `libmimalloc_sys`. Each bundles a static C library
  built by its build script, whose archive names and debug info held the checkout's path. Since
  `f9fb75060` those archives are byte-identical across checkouts (`-ffile-prefix-map`,
  `SOURCE_DATE_EPOCH`). Rerun: 378 of 378 hits, 142 s wall and 366 s CPU here (273 + 93) against
  170 s and 402 s.
- 142 s with every Rust crate a hit was still too long. `cargo --timings` showed where it went,
  and each step below was measured on the same pair of worktrees (cache filled by the first; load
  20–80):

  | Change | Second worktree, wall | What was still on the critical path |
  |---|---|---|
  | every Rust crate a hit | 142 s | 61 build scripts' C, uncached (SQLite 90 s); the engine's cdylib, uncached (47 s); 21 proc macros |
  | build scripts' C through the cache (`farm-cc`) | 114 s | the cdylib; the proc macros; the grammars' build script (23 s) |
  | crate types that link cached (`SCCACHE_CACHE_LINKED`) | 54 s | the grammars' build script; mimalloc (its C uses `__TIME__`) |
  | the grammars built a thread each, `SOURCE_DATE_EPOCH` kept | **28 s** | none: 562 of 562 compiles hit |

  Mistakes found on the way, each of which made a crate key differently per checkout or per run:
  a linker argument holding the target directory's path where sccache does not strip it, `xcrun`'s
  error text (with a time and pid) in the linker's identity, sccache dropping `SOURCE_DATE_EPOCH`,
  and the threaded grammars printing their link libraries in a different order each run. The CLI
  built from cached parts gives the same bytes on the smoke corpus as a plain cargo build.
- What is left of the 28 s is throughput: 521 units at about 0.9 s each, 36 at a time. A hit
  still starts the wrapper, hashes the sources and dependencies and writes the outputs, and each
  build script runs again, probing its compiler.
- Remote compiles cut this machine's CPU by 42% on a cold cache, but not the wall time under this
  load. Small crates pay ssh and tar per compile, and the workspace crates on the critical path
  compile here when the hosts are full.
- Mistakes found on the way. sccache's own probes (`--emit=dep-info`) must not go remote. One ssh
  connection serves only sshd's MaxSessions (10) and stalls the rest, so each slot has its own.
  A host's load average counts the farm's own compiles. rustc bundles `-l static` libraries into
  the rlib, so `-L native=` directories must be copied.
- The farm is on for every worktree: `setup` writes the machine's `~/.cargo/config.toml` (README,
  "One configuration").

#### Getting a compile's inputs to the host

A remote compile reads the checkout, its dependencies' `.rmeta` files and its `OUT_DIR`, and its
outputs must come back. Three ways, measured on the same build: `-p rerail-lang` with every compile
forced to miss (`SCCACHE_RECACHE=1`), `-j 36`, load about 70 from other sessions, 358 compiles
remote in each.

| Mode | Wall | CPU here | Moving files, per compile (mean, p90) | Compile there (mean) |
|---|---|---|---|---|
| copy: ssh and tar per compile, only what changed | 87.6 s | 377 s | 0.77 s, 1.60 s | 0.84 s |
| shared volume: the hosts mount the checkouts over NFS, no attribute cache | 404 s | 368 s | none | 7.3 s; 16 stale reads |
| live mirror: one open connection per host, only what changed | **76.5 s** | 364 s | **0.44 s, 1.05 s** | 0.96 s |

- NFS loses: each `stat` costs about 10 server calls, one per path component, and rustc stats a
  great deal. At this load a call takes 0.5–3 ms. With the attribute cache on, a host read a
  dependency directory listed before cargo wrote the new `.rmeta` (E0463). The farm now checks
  every file a compile read against this machine's and compiles here when one differs. The server
  tuning (16 nfsd threads at a higher priority, 1 s directory caching) is not yet measured.
- **Measured again (2026-11-15), with the server tuning on.** The compile's reads were replayed on
  each host, against its mirrored copy and over NFS: the dependency directory listed, then the
  engine's 219 dep-info inputs (7.6 MB) stat'ed and read, three rounds, at a load of 30–45 here.

  | Step | Mirrored copy | NFS |
  |---|---|---|
  | list the dependency directory (45,833 entries) | 33–140 ms | 1.8–9.8 s, with any mount options |
  | the same for 3,000 entries (a clean target's size) | | 35 ms; 1 ms within the 1 s cache |
  | stat the 219 inputs | under 1.5 ms | 50–450 ms (one call: 0.23–0.78 ms, the link's round trip at this load) |
  | read them | 5–15 ms | 0.3–3.1 s cold; 22 ms with attributes cached 1 s |

  The 7.3 s a compile took over NFS was mostly the listing: rustc lists each `-L dependency=`
  directory in every process, and this one had grown to 45,833 entries of stale artifacts (4,588
  crate names). Larger directory reads (`dsize`) and `rdirplus` did not change it. With a lean
  target directory and attributes cached for 1 s (the read check then catches what is stale), a
  compile would pay about 0.1–0.5 s, near the mirror's; with attributes uncached, the cold reads
  alone cost more than the mirror's whole sync. Not yet measured: a build end to end on a clean
  target directory, and writing the outputs over NFS.
- The mirror wins on moving files: no ssh session or tar per compile, and one scan of the checkout
  (11,298 files, 0.36 s) serves every compile that starts while it runs. The first compile on a
  fresh host sends the whole checkout (346 MB, 13 s). Its daemon used 5 s of CPU.
- In both copy modes the CPU here is about the same: cargo's own work (build scripts, proc macros,
  the binaries) and sccache dominate it, not the transfers.

- **End to end (2026-11-15).** The same `-p rerail-lang` build, every compile forced to miss, a
  fresh target directory each run, at a load of 30–70 here:

  | Mode | Wall | A remote compile | Fell back here |
  |---|---|---|---|
  | live mirror (over TCP) | 91 s | 0.55–0.73 s | 0 |
  | NFS, file attributes uncached, directories 1 s | 105–140 s | 1–3 s | 1–17 (E0463, E0460, E0432) |
  | NFS, nothing cached | 185 s | 4.3 s, mostly in rustc | 9 |

  A file created over the mount took 21 ms against 0.07 ms on the host's disk, a `.rmeta` written
  as rustc writes it 178 ms against 0.7 ms: the server commits each operation before it answers.
  So the shared volume was removed, with copy mode, and the mirror is the only way (README, "No
  shared volume, no copy mode"). The server's `async` setting, which answers first, was not tried.

#### What broke, and what fixed it (2026-11-15)

- **The hosts' own directories.** The mirror kept each file at this machine's path, and the
  shared volume's client moved a host's own `~/orca` aside to mount there: every session and
  workspace on both hosts lost its directory for a day. The mirror also wrote registry crates into
  the host's own `~/.cargo/registry/src`. Now everything the farm writes on a host is under one
  root of its own (`~/Library/Caches/rerail-farm/root.noindex`); the agent maps the paths in each
  compile and remaps them out of its outputs, and an output that still names the root (an
  `env!` path) compiles here (README, "The farm's root on a host").
- **Remote bytes are not local bytes under another path.** With the root, every remote output
  differed from the same compile here in about 226 bytes of hashes: rustc hashes each source
  file's real path into its output even when a remap hides it (shlex 1.3.0 compiled from two
  directories, both remapped to the same path: other bytes; another output directory: the same).
  A remote compile's output is cached and keyed on by every crate above it, so a crate compiled
  remotely in one worktree and here in another would split the cache. Now only crates of cargo's
  registry go remote, read in place from the host's own registry after a file-for-file check:
  the toy build's four remote crates (two hosts) were the same bytes as here.
- **Two writers in one target directory.** `test-remote`'s cargo built in the checkout's `target/`
  on the host, where the mirror also writes the `.rmeta` files of the compiles it runs. Two
  writers in one dependency directory gave E0463 and "failed to move dependency graph" in
  incremental builds. A fix in another worktree moved the target to a directory keyed by the
  checkout's name; its note said the crash was the mirror's copy being stale, which it was not, and
  two checkouts with one name would have shared a directory. Now each checkout gets a
  directory keyed by a hash of its path, outside the mirrored files.
- **Rebuilds skipped on the host.** The mirror gave each file this machine's mtime. A file that
  arrived after the host had built (an earlier send failed, or the host was cleaned) could be
  older than the host's build, and cargo there skipped it: a merge's 56 files had to be touched by
  hand. Now a written file's mtime there is the time of the write; this machine's mtime and size
  travel in an extended attribute, which is what the mirror compares.
- **A stale record of what a host holds.** After a host's directory was swapped under a running
  daemon (the restore above), the daemon still believed the old files were there, and
  `accel/src/lanes.rs` stayed at a two-day-old copy. Nothing but the farm writes under the
  root now, so nothing else can make the record wrong, and a failed write drops the agent and
  the record with it.
- **Two farm versions.** Scripts in an older checkout (`test-remote`) imported that checkout's
  farm while the daemon ran the installed one. Now every entry point runs the installed farm, a
  version number ties them together, and the daemon restarts when another version is installed
  (README, "One version").
- **Rebuilds when an environment differed.** A build script that exported `CC`, `CXX` and
  `JEMALLOC_OVERRIDE` made every other build of that worktree rerun the build scripts and rebuild
  the engine's tree (612 s). The machine's cargo configuration now forces the shared values, the
  wrapper refuses a build whose values differ, and the engine's build script refuses another
  `RUSTC_WRAPPER` (README, "One configuration"). Before, the farm's wrapper was opt-in, and
  every build without it used plain sccache, whose keys hold the checkout's path: no worktree
  reused another's compiles.
- **`test-remote --help` sent the checkout** (11,564 files) and ran `nice --help` on a host. Its
  options are now checked before anything is sent. It also sets the host's open-file limit to
  this machine's, which cargo nextest needs (EMFILE at the limit an ssh command gets by default), not a fixed
  value.

What else the idle machines can do:

- **Remote test runs.** This saves the most CPU here. Full suites and gates (45 CPU-minutes each)
  run on an idle machine over ssh, from a pushed branch, each machine keeping its own warm
  `target/`. The development machine keeps only edit, rebuild and targeted tests. Metal tests that
  need newer GPU features run only where the GPU has them.

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
5. **The farm for every worktree (§4), and the gates on the idle machines.**

### No benchmark leases (2026-11-16)

`quiet-run`'s leases (farm 9) are removed in farm 20. Its turns, drains, 10-minute cap and
fairness refusals made agents wait for an idle machine rather than work; with several test slots per
host (farm 19) the hosts are used for everything instead. `quiet-run` runs its command at once
through `test-remote`; a benchmark reports the load it ran under.
