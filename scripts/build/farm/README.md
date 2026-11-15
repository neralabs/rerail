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
  (the hosts of the local configuration, reached by ssh; see Configuration). The sources, the crate's dependencies' `.rmeta` files and
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
| `status` | what the farm can use now: each host probed, and why one is not usable |
| `host-privacy` | keeps the farm's files on the hosts out of Time Machine, and (`--spotlight`, with sudo) Spotlight |
| `setup` | installs all of it here, a configuration to fill in, and the toolchain on each host |
| `shared-volume` | the NFS export here and the mount on each host |
| `build-sccache` | builds the patched sccache from upstream and runs `setup` with it |
| `sccache-patches/` | the patches on upstream sccache: `0000` PRs #3021 and #3022, `0001` the farm's own |
| `farm.example.json` | a configuration to start from (generic names), copied to `~/.config/rerail-farm/farm.json` |

## Setup

```
scripts/build/farm/build-sccache     # the patched sccache, then setup with it
scripts/build/farm/setup             # setup alone (keeps the installed sccache)
```

The sccache is upstream at the commit `build-sccache` pins, with PRs #3021
(`SCCACHE_BASEDIRS_REMAP_PATHS`) and #3022 (`SCCACHE_BASEDIRS_VERBATIM_ENV_DEPS`) as merged on it
(`sccache-patches/0000`), and the farm's patch (`0001`). Its source and build stay in
`~/Library/Caches/rerail-farm/sccache-src`. Until the PRs land upstream, a newer base means
merging them again and exporting `0000` anew. The farm's patch caches
the crate types that link when `SCCACHE_CACHE_LINKED` is set (proc macros, build scripts, the
engine's cdylib: upstream never does), keyed also on the linker's and the SDK's versions; accepts
`--extern proc_macro` without a path; and keeps `SOURCE_DATE_EPOCH`, which upstream drops, so a C
file that uses `__TIME__` (mimalloc's) is cached. Nothing of the farm is installed on a host:
`remote-rustc` and `farm-agent` are sent with each command and run by the host's `/usr/bin/python3`.
Each host needs ssh without a password (its name in `farm.json` is a name ssh connects to: a `Host`
entry of `~/.ssh/config`), the toolchain of `rust-toolchain.toml` (`setup` installs it),
`cargo-nextest` for `scripts/dev/test-remote`, and, for copy mode only, a copy of
`~/.cargo/registry/src` at the same path:

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

## Configuration

The farm's configuration is local to this Mac and never in git: `$RERAIL_FARM_CONFIG` if set, else
`~/.config/rerail-farm/farm.json` (`farm.example.json` shows its shape). Without one there are no
hosts, and everything compiles and tests here, as without the farm.

| Key | Meaning |
|---|---|
| `hosts` | the hosts: `{"name", "cores", "enabled", "mirror", "tests", "shared", "link"}` |
| `name` | what ssh connects to (a `Host` entry of `~/.ssh/config`) |
| `enabled` | `false` leaves a host out (default `true`) |
| `mirror`, `shared` | how its compiles get their inputs (Live mirror, Shared volume; else copied) |
| `tests` | whether `scripts/dev/test-remote` runs there (default: `mirror`) |
| `link` | the host's network interface the farm's traffic must arrive on (its fast link); unset: any |
| `min_free_gb` | free disk a host needs (default 40) |
| `max_rtt_ms` | the slowest round trip a host may have (default 150), for a host without `link` |
| `min_mbps` | the least throughput to it, MB/s (default 8), for a host without `link`: with one, the link check catches a fall back to WiFi, and both are only reported |
| `tcp` | `false` keeps the live mirror's data on ssh (default `true`; also `RERAIL_FARM_TCP=0`) |

A host takes a compile per core while nobody has touched its keyboard or mouse for 5 minutes, and
half as many otherwise (`HIDIdleTime`). Compiles take slots in turn across the hosts (each host's
first slot, then each one's second), so every host works, not only the first listed. Not its load average: macOS's background analysis (Photos,
Spotlight) keeps that high on an idle Mac, at a priority every compile preempts.

### The hosts' backups and indexing

A host holds the farm's copies (`~/orca`, at this machine's paths), Relax and the caches. Left
alone, Time Machine backs them up and Spotlight indexes every file the mirror writes (over 200,000
on one host). `host-privacy` (run by `setup`) excludes them from Time Machine, which needs no
root. Spotlight ignores a folder's `.metadata_never_index` (macOS 15), and the paths cannot take the
`.noindex` suffix, so `host-privacy --spotlight` puts them in Spotlight's privacy list with sudo,
once per host, from a terminal. Its report counts what Spotlight still has indexed.

## Health

Hosts come and go: they sleep, leave the network, fall back to WiFi, fill their disk, or have their
copies cleaned. Nothing waits on one for long, and anything a host cannot do runs here.

- **Probe.** Before a host is used its health is read: one ssh command (3 s to connect) reports the
  interface the connection arrived on, its free disk, its idle time and its mounts; the round
  trip is timed, and every 10 minutes (or while it is slow) a 4 MB transfer measures the
  throughput. A host short of disk or past the limits is not used.
- **Off its link: recovered.** A host's name (often an mDNS `.local` name) resolves to whichever
  of its addresses answers first, so ssh can reach it over WiFi while its fast link is up. When
  a probe arrives on another interface than `link`, the farm asks the host, over the connection
  that works, for its addresses on `link`, tries each (3 s), and the first that arrives there
  becomes the host's route: every later ssh goes to that address, the shared connections made
  the other way are closed, and each route has its own. While no address works (the link is
  down) the host is not used, and is tried again every 60 s, not backed off. A route that stops
  answering is dropped, and the host is reached by name, then recovered again.
- **Cache and back-off.** A healthy record holds 30 s, shared by every worktree (one caller probes,
  the others read). A host that does not answer is not probed again for 30 s, doubling to 30
  minutes, so a sleeping host costs nothing; `status` probes every host now, whatever its
  back-off.
- **Lost mid-job.** A compile whose copy or run fails there runs here, and the host is marked down
  when its connection or agent is gone, or its disk is full (agent code 70). A test run whose ssh
  connection drops (exit 255) runs again here.
- **Cleaned copies.** The mirror remembers what it sent each host, and a host restarted or cleaned
  forgets: an agent is asked what it holds when it starts, a compile that read a stale file clears
  what was sent, and `test-remote` asks the host again for every file of the checkout before it runs.

`scripts/build/farm/status` probes every host now and prints whether it is usable and why not,
with its round trip, throughput and free disk (`--json` for scripts; exit 1 when none is usable).

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
come back on the stream.

The stream starts on ssh, then moves to plain TCP: the daemon listens on a port of its own,
sends the agent the port and a one-time token over ssh, and the agent connects back to the
address it sees ssh arrive from (the host's `link`, which the health check confirmed) and sends
the token. ssh's encryption and flow control capped the mirror at about 40–120 MB/s on a link
that carries 600 MB/s; over TCP a fresh 330 MB tree reaches a host at 220–330 MB/s, the host's
own disk writes included. ssh stays open as the lifeline: the agent ends when it does, and a
dead TCP connection is noticed in about 10 s (keepalive). If the agent cannot connect back
within 8 s, the stream stays on ssh. The data on the link is not encrypted: `tcp: false` keeps it
on ssh. Every file the compile read (its dep-info) must have the size and mtime
it has here, or the compile runs here. The agent refuses to write under an NFS mount or automount:
a host with the shared volume runs `shared-volume client-off` first.

`RERAIL_FARM_LOG=file` logs each decision (hot, hit: nothing logged, here, or which host) with
where a remote compile's time went, and why a compile did not go remote. `RERAIL_FARM_LOCAL=1`
compiles every miss here.
