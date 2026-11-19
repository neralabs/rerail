# Relax for rerail: the live test loop

## Intro

[Relax](https://github.com/makepad/relax) is a Rust compiler and JIT that runs a crate's tests
from source. In `--live` mode it keeps the tests running and patches each function body you
save into the running process, so a body edit reaches a verdict in 2–9 s. The same edit through
cargo is a rebuild of about 48 s (docs/final-shape/notes/t37-edit-loop-time.md, §3).

We use it for one thing: the inner loop of one area's tests. Everything else stays on cargo:
builds, the quick tier, the gate, wasm, release and CI. Relax has its own std, so a change is
committed only after the same tests pass under cargo.

Upstream Relax could not run the engine as it was, and every worktree's first run started cold.
We keep a small fork that fixes both. This page says what the fork changes and why, how to set
it up, and what we expect to hold.

## Setup

Build and install once, and again whenever `std.patch`, `rerail.patch` or the pinned commit
changes:

```
scripts/dev/relax/build          # the pinned, patched Relax, into ~/.local/share/rerail-relax
```

Then run a filter's tests live, from `native/`:

```
cd native && RUSTC_WRAPPER= RELAX_ALLOW_C=1 RELAX_SHARED_OPEN=1 RELAX_SHARE=rerail RELAX_CACHE_MAX=8192 \
  RELAX_STD=~/.local/share/rerail-relax/src/libs/relax/compiler/std \
  nice -n 19 ~/.local/share/rerail-relax/bin/relax test --live rerail-engine --lib -- FILTER
```

Drop `--live` for a single run. On a farm host: `scripts/dev/live-test-remote rerail-engine
--lib -- FILTER`.

What each setting does:

| Setting | Why |
|---|---|
| `RUSTC_WRAPPER=` | Relax compiles the native (C) dependencies with itself as the wrapper; the farm's wrapper would refuse its environment. |
| `RELAX_ALLOW_C=1` | The engine links C (tree-sitter, SQLite, zstd, simdjson, allocators). Without the grant no C is built or run. |
| `RELAX_SHARED_OPEN=1` | Shares registry crates' compiled code between every worktree's runs, in `~/.relax/code` (fork change 2). |
| `RELAX_SHARE=rerail` | Mounts the worktree at one path, `~/.relax/vfs/share/rerail`, so the workspace's own crates have the same keys in every worktree (see "Sharing between worktrees"). |
| `RELAX_CACHE_MAX=8192` | Caps the stores of the run dir at 8 GB, whatever the run's size (see "How the caches stay bounded"). |
| `RELAX_STD=…` | The patched std of the installed version (fork change 1). |
| `nice -n 19` | As for every build and test run (AGENTS.md). |

Other variables that help when something goes wrong:

| Variable | Effect |
|---|---|
| `RELAX_VERBOSE=1` | Phase times, cache hits and misses, and why a store went stale (`stale code store: … late input … changed`). |
| `RELAX_CACHE_TRACE=1` | With `RELAX_VERBOSE`: what each prune removed. |

Don't set `RELAX_CACHE_DIR`: it turns sharing off.

## How the build script installs

`scripts/dev/relax/build` keeps everything outside the worktrees:

| Path | What it holds |
|---|---|
| `~/Library/Caches/rerail-relax/repo.git` | A bare clone of upstream. |
| `~/Library/Caches/rerail-relax/target` | Cargo's target for building Relax itself. |
| `~/.local/share/rerail-relax/v/<version>/` | One patched source tree and its binary per version, never changed once built. |
| `~/.local/share/rerail-relax/src` | A link to the installed version's tree (`RELAX_STD` points into it). |
| `~/.local/share/rerail-relax/bin/relax` | The installed binary. |

A version is the pinned upstream commit plus a hash of both patches, for example
`5b2d68c26a51-8959825cbeab`. The script checks out the commit with `git worktree add`, applies
`std.patch`, then `rerail.patch`, and builds with the repository's toolchain
(`rust-toolchain.toml`).

It never switches versions under a running session: if a process of the installed binary is
running, it builds the new version and exits 75 without installing; `--now` installs anyway. A
live session would otherwise find its std's files changed under it.

## The fork

The fork is [neralabs/relax](https://github.com/neralabs/relax) (private for now), branch
`rerail`. `rerail.patch` is `git diff` from its first commit (the std items) to its last;
`std.patch` is the first commit alone. To change the fork: commit on `rerail`, push, regenerate
the patch (`git diff <std commit> rerail > scripts/dev/relax/rerail.patch`), run the build
script, and commit the patch here.

Relax's licence is MIT (the compiler crate is MIT OR Apache-2.0), so a fork is allowed.

The branch `wip/checkout-sets` holds an experiment that kept a store set per checkout. It did not
help: the stores were dropped for another reason (change 5). It is kept, not merged.

### 1. Std items the engine needs (`std.patch`)

Relax ships its own std, and upstream's lacked items the engine uses. When a run stops on a
missing std item, add the item to the patch, not a workaround to the engine.

### 2. Open runs share registry code (`RELAX_SHARED_OPEN=1`)

Upstream shares compiled code of "sealed" crates (registry crates whose dependencies have no
OUT_DIR) between checkouts, in `~/.relax/code`, but only in its sandboxed `makepad` runs. The
fork lets a plain `relax` run share too when `RELAX_SHARED_OPEN=1`. The cache's configuration
hash keeps these entries apart from sandboxed ones. Alone this saves about a tenth of a cold run:
the 50 MB of shared code is a small part of the store.

### 3. Build-script stubs are shell scripts

With `--allow-c`, cargo builds the native packages, and Relax gives it stand-ins for their build
scripts. Upstream made each one a hard link of Relax's own binary. Cargo removes and relinks
build scripts as it rebuilds them. macOS's syspolicyd then could not read the file by the name
it was scanning and killed the running Relax ("Terminating process due to Malware rejection").
The fork makes each stand-in `#!/bin/sh` / `exit 0`.

### 4. Borrow checking: a closure built inside a closure (E0310)

Relax rejected `ColMap::of_rel` (`plan/table.rs`) with E0310, which rustc accepts. The shape: an
edition 2024 `impl Iterator` returned from `fn f(&self, _r: &str)` (so it captures both
lifetimes) that holds a closure built inside another closure capturing `self`.

Relax maps each region slot of a closure type to an identity. When a slot is one of the parent's
own regions, the nested body's requirements map to that region. But the parent's instantiation
of the closure type had a fresh variable in that slot, and in the signature part nothing
constrained it. So the inner closure's borrow reached the hidden type unconstrained, and the
member constraints chose `'static`.

The fork ties those signature slots to the parent's regions, both ways, and only those. Tying
the upvar slots too broke `chunked.rs`'s `combine`: an or-pattern binding records only one of
its regions as the identity. Two tests in Relax's `mir_source_tests.rs` cover the accepted and
the rejected case, and Relax's 272 borrowck tests pass.

### 5. A build script's rerun key holds only what it reaches of the native build

This is the change that makes worktrees share. Before it, two worktrees whose code differed
undid each other's stores, and every switch cost what a run after an edit does.

The chain was:

1. The engine's build script compiles C, so it is a native package. The native build's key
   hashes a native path package's files, so any engine edit gives a new native build, in a new
   folder. The script also sets `RERAIL_ENGINE_BUILD_ID`, a hash of all engine sources.
2. Relax hands every build script it runs itself a "receipt": a file naming every native
   package's outputs and the native library's path. Nearly every script reaches some native
   package (Relax counts `libc` as one), so nearly every script got it. The receipt's path was
   part of each script's rerun key.
3. So after any engine edit, every such script ran again, into a new OUT_DIR. A crate store
   records the `env!`/`include!` inputs of the crate's whole dependency closure (its ledger),
   and an OUT_DIR is one. Every crate above `unicode_names2` (24 stores) went stale.

The fork gives each script `RELAX_NATIVE_RECEIPT_ID` and keys on that instead of the receipt's
path. The identity covers:
- the outputs of the native packages this script's build-dependencies reach (cfgs, env, OUT_DIR,
  system libraries);
- the files of any of those packages that are path packages;
- the library's archives and packages.

It leaves out the library's path. Every list is taken as a set. The link lists the archives in no
fixed order, and two native builds at once list a package's host and target runs in either
order. Both orderings were each caught by a test run that showed the stores dropped again. The
script still gets the full receipt; only the key narrows.

Tests: `a_scripts_key_holds_the_receipts_identity_not_its_path` (unitgraph.rs) and
`a_receipt_identity_is_of_the_reached_packages_as_sets` (native.rs).

## Sharing between worktrees

Relax keys a run's stores by the workspace's absolute paths. Under `RELAX_SHARE=rerail` every
worktree is mounted at the same virtual path, so the keys match, and with change 5 an edit in one
worktree no longer invalidates the other's crates.

Measured on one session test (`a_session_keeps_a_negation_that_still_fails`), on the M5 Max at a
load of 35–75. The full tables are in t37.

| Case | Wall | CPU |
|---|---|---|
| A worktree's first run of code another has run | 15 s | 27 s |
| Taking turns between two worktrees whose code differs | 26–40 s | 97–114 s |
| The same turns before change 5 | 115–183 s | 242–272 s |
| Two cold runs at once | 152 s, 156 s | 471 s, 328 s |
| The same two runs again, at once | 28 s, 70 s | — |
| Same code, after the fixtures fix: C after A, then A | 10 s, 6 s | 27 s, 19 s |
| Two `--live` sessions side by side: first verdicts | 110 s, 111 s | — |
| … then a body edit saved in one (the other sees nothing) | 3–10 s to its verdict | — |
| `live-test-remote` on the M3 Max, cold in its new home: first verdict | 160 s | — |
| … then a body edit saved here | 2–3 s to its verdict | — |
| One cold run, a new Relax build | 87–157 s | 354–409 s |

What a switch between different code still redoes:
- the engine crate's own store, because its build ID (`RERAIL_ENGINE_BUILD_ID`) hashes all its
  sources. (Its build script no longer writes the checkout's path into `RERAIL_FIXTURES_DIR`:
  without an override that comes from `.cargo/config.toml`, the same path under the mount.)
- the native cargo step, about 7 s, because a slot keeps only its last native build.

Two live sessions, one per worktree, run side by side: each patches the edits saved in its own
checkout, though both are mounted at one path, and the other's session sees no event.

## How the caches stay bounded

Under `RELAX_SHARE=rerail` the stores live in one root, `~/.relax/target/<key>`. Without it each
worktree has its own, `<worktree>/target/relax`.

| What | Where | Bound |
|---|---|---|
| Crate and code stores | `<root>/cache` | At each start, older generations go, then whole store files, least recently used first, while the folder exceeds the cap. The cap is `RELAX_CACHE_MAX` MB, else twice what the run uses, at least 4 GB. It never deletes a store a running process holds or another run used in the last 10 minutes. |
| Replaced generations | `<root>/cache/*.rcs.N` | Deleted at exit, except one a reader still holds. |
| Shared registry code | `~/.relax/code` | Its own cap, the same rule. |
| Native builds | `<root>/native` | By reachability: what each slot's last build uses stays; the rest goes. |
| Build-script outputs | `<root>/build` | By reachability, the same way. |
| Installed versions | `~/.local/share/rerail-relax/v/` | The build script keeps the installed version and the one before it (about 110 MB each), and any a process runs from. Not `v/old-src`, an earlier script's checkout that can hold edits. |
| Relax's own build | `~/Library/Caches/rerail-relax/target` | `clean-target` after each build of Relax. |
| Run dirs no run uses | `<worktree>/target/relax`, other `~/.relax/target/<key>` | `clean-target` (the farm runs it every 15 minutes per target directory) removes one idle for 6 hours (`--relax-keep-hours`), whole. Kept: the most recently used shared run dir, whatever its age, and any a Relax process runs in (each holds a lock in `cache/live`). It renames the dir aside under those locks before deleting it, so a Relax that starts meanwhile makes a fresh one. |

Measured on 2026-11-15: `~/.relax` 9.6 GB (stores 8.4 GB, native builds 1.0 GB, shared code
110 MB). Per-worktree roots from before sharing: 9.0 GB and 3.9 GB. `relax clean` removes a
worktree's `target/relax` at once. `scripts/build/farm/diagnose` shows each run dir, its size and
last use, and warns when a checkout's Relax runs use their own run dir (not shared).

So the total is bounded: the shared run dir's stores at `RELAX_CACHE_MAX` (8 GB) plus its native
builds (about 1 GB, by reachability), the shared code under the same cap (now 110 MB), two
installed versions, and at most 6 hours of any other run dir.

## Invariants

These must hold. A change to the fork, the patches or the commands that breaks one is a bug.

1. **A Relax pass is not a cargo pass.** Commit only after the same tests pass under cargo.
2. **One build per version.** A version's tree and binary never change once built. Store names
   hash Relax's codegen identity, so a new build starts with cold stores and never reads an old
   build's.
3. **No switch under a running session.** The build script installs only when no process of the
   installed binary runs (or with `--now`).
4. **Same code, same keys, in every worktree.** Under `RELAX_SHARE=rerail` a worktree's first
   run of code another has run is warm: about 15 s, not a cold run.
5. **An edit invalidates only what reads it.** An edit in one crate makes stale only that
   crate's stores, the crates that depend on it, and the crates whose `env!`/`include!` inputs
   it changes. It never reruns a build script that does not reach the edited package. Check with
   `RELAX_VERBOSE=1`: after an engine edit, the only `stale code store` line names
   `rerail_engine`.
6. **Keys are order-free.** No list a key hashes depends on the order a tool produced it in
   (link order, host/target order, directory order).
7. **Concurrent runs are safe and share.** Two runs at once in different worktrees both pass. One
   waits on the other's build-script claim and takes its outputs instead of running the script
   twice. Neither deletes a store the other holds. Two live sessions side by side each patch only
   their own checkout's edits.
8. **The caches stay bounded.** Each run dir's stores stay under their cap; native builds and
   script outputs are kept by reachability alone; a run dir no run uses goes after 6 idle hours;
   two installed versions stay. Nothing is removed while a Relax process uses it.
9. **The engine has no Relax workarounds.** A missing std item goes into `std.patch`, a Relax bug
   into the fork. The engine's code stays what cargo compiles.
10. **No machine details in the tree.** Model numbers only. No hostnames, usernames, home
    paths or network layout in the patches, this page or commit messages.

## Known limits

- A live session holds about 6.5–7.5 GB.
- A change in the set of native packages (a new `-sys` crate or grammar) moves every native
  package's OUT_DIR (cargo's target for the selection is named by the set), so every build script
  that reaches one runs again once, and the crates above them recompile.
- `live-test-remote` runs Relax on the host as here (`RELAX_SHARE`, `RELAX_SHARED_OPEN`,
  `RELAX_CACHE_MAX`), with its home under the host's farm root
  (`<farm root>/<this machine's ~/Library/Caches/rerail-farm/relax-home>`), never the host's
  `~/.relax`. `clean-target` does not run on hosts; there the cap bounds the live run dir, and
  the farm's own prune keeps the disk free.
- Relax checks only the functions a test reaches. A variant that a test never calls compiles
  "fine" under Relax.
