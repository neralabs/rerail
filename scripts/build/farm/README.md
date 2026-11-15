# The build farm

One compiler cache for every Rerail worktree on this Mac, and the other Macs for what it misses.

## Intro

Most worktrees edit one crate. Each still compiled every other crate itself, because a compiler cache
keyed on the checkout's path saw each worktree's crates as different. The farm fixes both halves:

- **Sharing.** Every compile goes through one sccache server (port 4227, its own cache in
  `~/Library/Caches/rerail-farm/sccache`). It is a patched sccache: the worktrees are its *basedirs*,
  so a crate's key holds paths relative to its checkout, and `--remap-path-prefix` makes its outputs
  the same in every checkout. A crate compiled in one worktree is a hit in all the others.
  This needs the engine not to write its checkout's path into its build (decision 0224).
- **Spreading.** On a miss, a dependency's library crate compiles on another Mac when one has a free core
  (the hosts in `~/.config/rerail-farm/hosts.json`, reached by ssh). The sources, the crate's dependencies' `.rmeta` files and
  its `OUT_DIR` are copied to the same absolute paths there, only what changed since the last copy,
  and the outputs are copied back.
- **Editing.** A workspace crate that misses is being edited here: it compiles incrementally on
  this Mac, never on another, so its next edit starts from that state. For the next 2 hours it
  skips the cache, as without the farm. A crate that was a hit has no incremental state: its first
  edit compiles it whole.

```
cargo ──▶ rustc-wrapper ──┬─ hot crate ───────────────▶ rustc -C incremental (here)
                          └─ else ─▶ sccache :4227 ──┬─ hit ─▶ outputs from the shared cache
                                                     └─ miss ─▶ farm-rustc ─┬─ lib ─▶ ssh a host     
                                                                            └─ else ─▶ rustc (here)
```

What is never remote: build scripts, proc macros, binaries and tests (they link), and any compile
whose copy or run failed there: it runs here instead, and only that run's output is shown.

## Files

| File | Role |
|---|---|
| `rustc-wrapper` | cargo's `RUSTC_WRAPPER` |
| `farm-rustc` | the compiler the farm's sccache runs on a miss |
| `farm-cc` | build scripts' C and C++ compiler (`CC`; `farm-c++` for `CXX`): clang through the farm's sccache |
| `farm.py` | all of them |
| `remote-rustc` | runs one compile on a remote Mac (copy and shared modes) |
| `mirror.py` | the live mirror's daemon here, and the call a compile makes to it |
| `farm-agent` | the live mirror's end on each host |
| `setup` | installs all of it, and `remote-rustc` on each host |
| `shared-volume` | the NFS export here and the mount on each host |
| `sccache-patches/` | the farm's patch on sccache, on top of #3021 and #3022 |
| `hosts.example.json` | the hosts, copied to `~/.config/rerail-farm/hosts.json` |

## Setup

```
scripts/build/farm/setup path/to/patched/sccache
```

The sccache is upstream main with PRs #3021 (`SCCACHE_BASEDIRS_REMAP_PATHS`) and #3022
(`SCCACHE_BASEDIRS_VERBATIM_ENV_DEPS`) merged in, and `sccache-patches/` applied. The patch caches
the crate types that link when `SCCACHE_CACHE_LINKED` is set (proc macros, build scripts, the
engine's cdylib: upstream never does), keyed also on the linker's and the SDK's versions; accepts
`--extern proc_macro` without a path; and keeps `SOURCE_DATE_EPOCH`, which upstream drops, so a C
file that uses `__TIME__` (mimalloc's) is cached. Each host needs ssh without a password (its name in
`hosts.json` is a name ssh connects to: a `Host` entry of `~/.ssh/config`), the
toolchain of `rust-toolchain.toml`, and a copy of `~/.cargo/registry/src` at the same path:

```
rsync -a ~/.cargo/registry/src/ HOST:.cargo/registry/src/
```

Then point cargo at the wrapper, in a `.cargo/config.toml` in a directory above the worktrees
(an absolute path: cargo does not expand `~`):

```toml
[build]
rustc-wrapper = "<home>/.local/share/rerail-farm/rustc-wrapper"

[env]
CC = "<home>/.local/share/rerail-farm/farm-cc"
CXX = "<home>/.local/share/rerail-farm/farm-c++"
```

Without `CC` and `CXX` every new target directory compiles the build scripts' C again (SQLite's
alone takes 90 s at a load of 80).

`JEMALLOC_OVERRIDE` set to a prebuilt `libjemalloc_pic.a` saves `tikv-jemalloc-sys`'s 3-minute
configure-and-make in every new target directory; its build script is never cached.

## Hosts

`hosts.json` lists `{"name", "cores", "enabled", "shared"}`. A host takes a compile per core while
nobody has touched its keyboard or mouse for 5 minutes, and half as many otherwise (`HIDIdleTime`,
read at most every 30 s). Not its load average: macOS's background analysis (Photos, Spotlight)
keeps that high on an idle Mac, at a priority every compile preempts. A host behind a slow link is
better disabled.

## Shared volume

Without one, each remote compile copies what changed: the checkout's files (by size and mtime),
the dependencies' `.rmeta` files and `OUT_DIR`, then the outputs back. That costs about as much as
the compile itself (0.8 s a job: 0.5 s for the dependencies, 0.25 s for the outputs). With
`"shared": true` a host mounts this machine's `~/orca` (the checkouts and their target
directories) at the same path over NFS, and nothing is copied: rustc reads its inputs and writes
its outputs in place.

```
scripts/build/farm/shared-volume server ADDRESS PREFIX     # here
scripts/build/farm/shared-volume client ADDRESS SERVER     # on each host
```

Each gives the link between the machines (the network service `FARM_NETWORK_SERVICE` names) a static address on a
private subnet of its own, and the export reaches only that subnet. The hosts mount with attribute
caching off, so a dependency's `.rmeta` written here is seen there at once. cargo's registry
sources stay copied (`setup`); a host's own `~/.cargo` is its own.

A crate that links a host's system library that the host lacks (Xcode 26's `clang_rt` on an older
macOS) fails there and compiles here.

## Live mirror

With `"mirror": true` a host keeps a copy of what its compiles read on its own disk, at this
machine's paths, and is sent only what changed, over one connection that stays open. A daemon here
(`mirror.py`, started by the first compile, gone after 15 idle minutes) holds a `farm-agent` per
host over ssh. For each compile it scans what the compile may read (the checkout's files, the
dependency directory, `OUT_DIR`, native library directories; one scan serves every compile that
started before it), sends each file whose size or mtime differs from what it last sent, then the
compile, on the same stream: the agent writes the files before it runs the compile. The outputs
come back on the stream. Every file the compile read (its dep-info) must have the size and mtime
it has here, or the compile runs here. The agent refuses to write under an NFS mount or automount:
a host with the shared volume runs `shared-volume client-off` first.

`RERAIL_FARM_LOG=file` logs each decision (hot, hit: nothing logged, here, or which host) with
where a remote compile's time went, and why a compile did not go remote. `RERAIL_FARM_LOCAL=1`
compiles every miss here.
