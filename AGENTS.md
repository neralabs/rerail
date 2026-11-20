# Agent instructions

## Builds and test runs

Run every build and test run at a high nice value, so it never starves other work on the
machine:

```
nice -n 19 cargo build …
nice -n 19 cargo test …
nice -n 19 ../target/debug/deps/rerail_engine-… <filter>
```

This holds for background runs too, and for any long command a script starts.

Build only the target the next step needs. The engine's unit tests live in its library, so
its test target compiles the whole crate a second time:

| Target | Measured (2026-11-14, O0, after a one-file edit in `logic`) |
|---|---|
| The CLI alone (`--bin rerail-engine`) | 19 s wall, 22 s CPU |
| The library's tests (`--lib --tests`) | 18 s wall, 26 s CPU (a `facts` edit: 31 s, 52 s) |

So a run of the CLI builds `--bin rerail-engine` only, and the test target is built when
tests are run, not with every build.

Build the CLI for development with the tests' features:

```
nice -n 19 cargo build --bin rerail-engine --features test-hooks
```

On a machine with the build farm (below), build it on a farm host instead, and bring the program
back (`--fetch`, a path in the target directory; it keeps the build on a Mac):

```
scripts/dev/test-remote --fetch debug/rerail-engine -- cargo build --bin rerail-engine --features test-hooks
```

This Mac runs the edit loop and many sessions at once, at loads of 30–90; an idle host compiles
an edit to the engine in about 8 s (t37 §4). With no usable host the build runs here, into
`target/` as above. A rebuild with nothing changed takes about 6 s, the fetch included.

The tests turn on `test-hooks` in every dependency crate (and a few registry features). A CLI
build without it compiles each of those crates a second time, apart from the tests' build, and
keeps a second set of incremental state: twice the CPU per edit, and about 2 GB of disk per crate
variant. With it, an edit compiles each dependency crate once for both, and a crate's own tests
(`cargo test -p rerail-logic`) build the crates below it the same way too. Product builds (release,
wasm, the Node addon) and measurement builds (`--profile profiling`) stay without it.

Keep to that one feature set in this worktree's target. The farm's wrapper tells any build that
compiles a workspace crate with other features than a build of the last 2 hours did, names the
features and the earlier command, and repeats it at the checkout's next builds for 3 hours. Run a
build with other features (`metal-tests`, `headless`, `code-rule-profile`, `--no-default-features`)
on a farm host with `scripts/dev/test-remote`. `scripts/check/check-dev-features.py` (pre-commit)
keeps the manifests' features in line: a crate's `test-hooks` and `default` turn on those of the
crates below it.

The farm's wrapper runs `scripts/dev/clean-target` on each target directory every 15 minutes: it removes the
builds no current build was made from, debug objects no binary names, and stale incremental state.
Never run a full `cargo clean`. Under 200 GB of free disk on this Mac, the farm removes the least recently built
worktrees' build outputs on its own (`scripts/build/farm/free-local`; the farm's README, "This Mac's disk floor"): a
build of an idle worktree may then start from incremental state lost, or from nothing.

### Test tiers (cargo-nextest)

`.config/nextest.toml` splits the library's tests in two tiers (docs/final-shape/notes/t37, §2).
From `native/`:

```
nice -n 19 cargo nextest run --lib -E 'test(/some::filter/)'   # targeted: the usual run
nice -n 19 cargo nextest run --lib                             # the quick tier: all but the slow tests
nice -n 19 cargo nextest run --lib --profile gate              # everything: a phase gate only
nice -n 19 cargo nextest run --lib --ignore-default-filter NAME  # one slow test by name
```

On a machine with the build farm (below), run these through `scripts/dev/test-remote -- cargo nextest
run …` by default: the hosts take several test commands each at once, in a warm target directory per
checkout, and this machine stays free for the edit loop. A build here prints when a host has a free
test slot. Run tests here only when no host is usable (test-remote then runs them here itself).
A command that does not need macOS may go to a Linux host, as a Linux build and run (as CI's):
`--os macos` keeps one on a Mac when its result must be macOS's. Each command gets a share of its
host (twice its cores over its test slots) as its cargo jobs and test threads.

A test that takes 20 s or more belongs in the slow tier (the default profile's
`default-filter`). Tests that read the packs' test data need it pulled first
(`npm run fixtures:pull`); without it they fail, which says nothing about the change.

### Other Macs: remote tests and the build farm

Some machines have a build farm (`scripts/build/farm/README.md`): other Macs that run test
commands and compile dependency crates. Everything about those machines lives in a local file
outside git: `$RERAIL_FARM_CONFIG`, else `~/.config/rerail-farm/farm.json`. Never copy any of it
into the tree, a commit, a log in the tree or a decision record: no hostnames, ssh names,
usernames, home paths, addresses, interfaces or network layout. Model numbers are fine. Without
the file there are no hosts, and every command below runs here instead.

```
scripts/build/farm/status                          # which hosts are usable now, and why not
scripts/build/farm/dashboard --open                # a live page: each host's CPU, GPU, memory, disk, jobs, links
scripts/build/farm/diagnose [DIR | --all]          # why a checkout's builds miss the shared cache
scripts/dev/test-remote -- cargo nextest run --lib --profile gate
scripts/dev/test-remote -- cargo nextest run --lib -E 'test(/some::filter/)'
scripts/dev/live-test-remote PACKAGE --lib -- FILTER   # Relax's live loop on a host
```

Benchmarks are ordinary commands: run them with `test-remote` (`--tree DIR` sends their inputs or a
program built here) or here, without waiting for an idle machine, and note the machine's load with
the result. There are no benchmark leases.

`test-remote` sends this checkout's changes to a usable host and runs the command there, from
`native/` (or `--dir`), in a warm target directory of its own on that host. A host can be asleep, gone,
on a slow link, or short of disk, or its copy can have been cleaned. The scripts check for this
themselves: a host reached over the wrong link is moved back to its fast one when it can
be, a host that fails a probe is skipped, with a growing back-off, and a lost connection
reruns the command here at `nice -n 19`. Don't work around a host that is down. Run
`status` to see why, and go on locally. Never wait for a host to be idle or less loaded: a
usable host takes the command whatever its load, and `test-remote` itself waits only for a
free test slot. Its busy CPU and load average in `status` are information, not a gate. `--remote-only` exits 69 when no host can take the
command. Several commands of one checkout may run at once: on one host they share its copy of the
checkout when their files are the same, and otherwise one waits for the other or takes another
host, so a command never reads another's files. A host may be a Linux machine: it takes a command as a Mac does (never a
Metal one, a `--tree` run or a `--fetch`), and the command is then a Linux build and run, as CI's; its line
says `(Linux: ...)`, and `--os macos` keeps a command on a Mac. Some hosts are VMs (guests) that run only
while used: a command that would wait for a slot starts one, and takes it once it is up (seconds to a
minute), unless a slot frees first; idle guests stop on their own (`scripts/build/farm/guests`). It also compiles this Mac's library
crates, in a container, to the very bytes this Mac would (the farm's README, "Mac compiles on a Linux host"). Both scripts run the installed farm (`~/.local/share/rerail-farm`), whatever version
this checkout has.

On a machine with the farm, every build already goes through it: `~/.cargo/config.toml` (written
by `scripts/build/farm/setup`) sets the farm's wrapper and the environment every build shares, and
then a crate one worktree compiled is a cache hit in every other one. So don't export
`RUSTC_WRAPPER`, `CC`, `CXX`, `SOURCE_DATE_EPOCH` or `JEMALLOC_OVERRIDE` in a script or command: a
build whose environment differs would rebuild and cache apart, and the farm stops it with an
error that names the variable (`RERAIL_FARM_ANY_ENV=1` allows one that must differ).
`RERAIL_FARM_LOCAL=1` keeps every compile here, and `RERAIL_FARM_LOG=FILE` logs where each
compile ran.

### Relax: the live test loop

[Relax](https://github.com/makepad/relax) (our fork and its setup: `scripts/dev/relax/README.md`) runs the engine's tests from source and patches each
saved function body into the running tests: a body edit reaches a verdict in 2–9 s, against
about 48 s for a cargo rebuild (t37, §3). Use it for the inner loop of one area's tests. Keep
cargo for everything else: builds, the quick tier, the gate, wasm, release and CI. Commit only
after the same tests pass under cargo, because Relax's std is its own.

```
scripts/dev/relax/build        # once, and after it changes: the pinned, patched Relax, into ~/.local/share/rerail-relax
cd native && RUSTC_WRAPPER= RELAX_ALLOW_C=1 RELAX_SHARED_OPEN=1 RELAX_SHARE=rerail RELAX_CACHE_MAX=8192 RELAX_STD=~/.local/share/rerail-relax/src/libs/relax/compiler/std \
  nice -n 19 ~/.local/share/rerail-relax/bin/relax test --live rerail-engine --lib -- FILTER
scripts/dev/live-test-remote rerail-engine --lib -- FILTER    # the same, on a farm host (a Mac first, else a Linux one)
```

The engine needs std items that upstream Relax lacked. They are `scripts/dev/relax/std.patch`,
and the build script applies it. When a Relax run stops on a missing std item, add the item to
the patch, not to the engine. `scripts/dev/relax/rerail.patch` lets a run share registry crates'
compiled code with every worktree's runs (`RELAX_SHARED_OPEN=1`, in `~/.relax/code`), and makes
build-script stubs plain scripts: as hard links of Relax's binary, cargo's replacing them got
Relax killed by macOS ("Malware rejection"). `RELAX_SHARE=rerail` mounts the worktree at one
path, `~/.relax/vfs/share/rerail`, so every worktree's runs share the workspace's crates too: a
worktree's first run of code another has run takes about 15 s, and runs in two worktrees
whose code differs take turns at 25–40 s. Without it a worktree's first run is cold (5–10 min). The patch
keys a build script's rerun on the native outputs it reaches, not the whole native receipt, so
an engine edit no longer reruns every script and drops the stores of every crate above them. The build
script does not switch versions while a Relax session runs. A live session holds about 6 GB. The shared run dir's stores
stay under `RELAX_CACHE_MAX` (8 GB), and `clean-target` removes a run dir no run has used for 6
hours (scripts/dev/relax/README.md, "How the caches stay bounded").

### Special builds

- **sccache for the farm** is patched (upstream PRs #3021 and #3022, and the farm's own patch in
  `scripts/build/farm/sccache-patches/`). `scripts/build/farm/build-sccache` builds it from a
  pinned upstream commit and installs it. The global cargo configuration's plain sccache needs no
  patch.
- **Relax** is pinned and patched: `scripts/dev/relax/build` (above).
- Both build outside every worktree, under `~/Library/Caches`, with the repository's toolchain.
  Don't build them in a worktree's `tmp/`, and don't run binaries from there.

`rerail-rr-aot`'s generated tiers (about 700 KB of Rust) are written by its build script only
with the `generated` feature, so a workspace build leaves them out. To test them:
`nice -n 19 cargo test -p rerail-rr-aot --features generated --profile profiling`.

## Merging branches

Read [docs/merging.md](docs/merging.md) before resolving any merge. In short:

- Before resolving, read both sides' commits and the decision records each side added since the
  merge base. They are the invariants the result must keep.
- Resolve hunk by hunk, by invariant. Never take one side wholesale (`--ours`, `--theirs`,
  `-X ours`, copying a file over) for a format, constant, grammar, schema, or any file both sides
  changed in meaning. A shared definition stays one definition.
- Keep each side's added tests and checks, ported to the new code if it was rewritten.
- Renumber colliding decision records yourself, past the highest number in any worktree.
- Before committing, run `scripts/check/merge-audit.py`. Restore each finding, or name it in the
  merge commit as superseded, with the commit or decision that supersedes it.
- The merge commit says which invariant each contested resolution keeps. "Kept theirs" is not a
  reason.
- Then run the targeted tests that both sides added or changed.
