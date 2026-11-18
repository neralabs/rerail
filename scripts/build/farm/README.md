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
- **Spreading.** On a miss, a library crate from cargo's registry compiles on another Mac when one
  has a free core (the hosts of the local configuration, reached by ssh; see Configuration). It
  reads its sources from the host's own registry, in place, after a check that they are the same
  files; the live mirror sends its dependencies' `.rmeta` files into the host's farm root, only
  what changed since the last send, and the outputs come back, the same bytes as a compile here.
- **One configuration, one version.** Every Rerail build on this Mac has the same wrapper and the
  same environment, from `~/.cargo/config.toml`, and a build that differs is stopped, not quietly
  compiled apart. Every worktree runs the one installed farm.
- **Editing.** A workspace crate that misses is being edited here: it compiles incrementally on
  this Mac, never on another, so its next edit starts from that state. For the next 2 hours it
  skips the cache, as without the farm. A crate that was a hit has no incremental state: its first
  edit compiles it whole.

```
cargo ──▶ rustc-wrapper ──┬─ hot crate ───────────────▶ rustc -C incremental (here)
                          └─ else ─▶ sccache :4227 ──┬─ hit ─▶ outputs from the shared cache
                                                     └─ miss ─▶ farm-rustc ─┬─ lib ─▶ mirror ─▶ a host
                                                                            └─ else ─▶ rustc (here)
```

What is never remote: build scripts, proc macros, binaries and tests (they link), the workspace's
own crates and any crate whose build script wrote sources (they are this machine's paths only:
see "The farm's root on a host"), a registry crate the host lacks or has otherwise, and any
compile whose send or run failed there, or whose outputs named the host's root: it runs here
instead, and only that run's output is shown.

## Files

| File | Role |
|---|---|
| `rustc-wrapper` | cargo's `RUSTC_WRAPPER` |
| `farm-rustc` | the compiler the farm's sccache runs on a miss |
| `farm-cc` | build scripts' C and C++ compiler (`CC`; `farm-c++` for `CXX`): clang through the farm's sccache |
| `farm.py` | all of them |
| `mirror.py` | the live mirror's daemon here, and the call a compile makes to it |
| `farm-agent` | the live mirror's end on each host, and (`exec`) a test command there |
| `test-mirror` | a test of the mirror against an agent run here: a file the checkout deleted goes from the host's copy |
| `status` | what the farm can use now: each host probed, and why one is not usable |
| `host-privacy` | keeps the farm's files on the hosts out of Time Machine, and (`--spotlight`, with sudo) Spotlight |
| `setup` | installs all of it here, the machine's cargo configuration, a configuration to fill in, and the toolchain on each host |
| `cargo-config` | writes the farm's part of `~/.cargo/config.toml` (`--check` reports) |
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
`farm-agent` is sent with each command and run by the host's `/usr/bin/python3`.
Each host needs ssh without a password (its name in `farm.json` is a name ssh connects to: a `Host`
entry of `~/.ssh/config`), the toolchain of `rust-toolchain.toml` (`setup` installs it) and
`cargo-nextest` for `scripts/dev/test-remote`.

## One configuration

A compile is reused across worktrees only when it is the same compile: the same wrapper (the
farm's, whose sccache keys a crate by its paths relative to its checkout), and the same
environment for its build scripts. A build script reruns, and every crate above it rebuilds and
keys apart, when a variable it reads changes: `CC` set by one script and not by another, or
`JEMALLOC_OVERRIDE` (a 612 s rebuild of the engine's tree, 2026-11-15). So there is one place for
them, the machine's `~/.cargo/config.toml`, which every checkout reads wherever it is (`setup` runs
`cargo-config`, which writes it and keeps the file's other keys):

```toml
[build]
rustc-wrapper = "<home>/.local/share/rerail-farm/rustc-wrapper"

[env]
CC = { value = "<home>/.local/share/rerail-farm/farm-cc", force = true }
CXX = { value = "<home>/.local/share/rerail-farm/farm-c++", force = true }
SOURCE_DATE_EPOCH = { value = "0", force = true }
```

`force` makes cargo's value win over one a script exports. Other projects read the same file: the
wrapper sends their compiles to plain sccache and `farm-cc` to clang, as before. A Rerail build
that still differs is stopped with an error that names the difference:

- The wrapper refuses a compile whose environment is not `farm.CANONICAL` (`CC`, `CXX`,
  `SOURCE_DATE_EPOCH`), or that has one of `farm.FORBIDDEN` set (`JEMALLOC_OVERRIDE`: it links a
  prebuilt jemalloc that a version bump in Cargo.lock would not notice). A forced value cannot
  differ at all. `SOURCE_DATE_EPOCH` is not forced: Rerail's own `.cargo/config.toml` sets it as a
  plain string, and cargo cannot merge that with a forced one, so every build there would fail.
  A forbidden variable has no value to force (an empty `JEMALLOC_OVERRIDE` is a missing file to
  its build script). The refusal comes at the build's first compile, but cargo reruns a build
  script whose binary is built without calling rustc, so that build script has run once with
  the variable: one rebuild follows, and the build fails, rather than every later build quietly
  rebuilding. cargo's own probes (`--print`) are never refused: cargo keeps their output, a
  failure included, in `target/.rustc_info.json`, and would repeat the refusal after its cause
  was gone.
- The engine's build script refuses a build whose `RUSTC_WRAPPER` is not the farm's, on a machine
  with the farm's configuration: an exported `RUSTC_WRAPPER` overrides the configuration's, and
  then the farm's wrapper never runs to check. Relax (`RELAX_STD` set) builds without one.
- `RERAIL_FARM_ANY_ENV=1` allows a build that must differ (a measurement of the compiler).

Without `CC` and `CXX` every new target directory compiles the build scripts' C again (SQLite's
alone takes 90 s at a load of 80).

## One version

The daemon, the wrapper and the agents on the hosts are the installed farm
(`~/.local/share/rerail-farm`), one version for every worktree; `test-remote` and
`live-test-remote` run it too, not their checkout's copy, and say when their checkout's is newer.
`farm.VERSION` numbers it: `setup` refuses to install an older version over a newer one (`--force`
does), the daemon refuses a caller of another version, and stops taking compiles once `setup`
installs another (it finishes the ones it has; the next compile starts the new one). A change to
the farm's protocol or to what a host keeps raises `VERSION`.

## Old builds

Rerail's worktrees together write terabytes of builds a day: each edit of a workspace crate leaves
the previous build, its debug objects and its incremental state behind, and cargo never removes
them. So the wrapper starts `scripts/dev/clean-target --wait` (installed beside it) on a build's
target directory, at most once every 15 minutes per directory, detached and at nice 19. It waits for that
build to end (cargo's lock on the profile directory), then removes what no build can use any more:
another rustc's artifacts, builds that none of the current builds was made from (a mark and sweep
over cargo's own dependency records, an hour's grace), debug objects no binary names, incremental
state of crates no longer built or untouched for 2 hours, and Relax run dirs no run has used for
6 hours (a worktree's own `target/relax`, another share's, never the most recently used shared one
nor one a Relax process runs in: scripts/dev/relax/README.md). Each target
directory's runs are logged in `~/Library/Caches/rerail-farm/tidy/<key>.log`.
`RERAIL_FARM_NO_TIDY=1` keeps a build from starting one.

A host's target directories (`test-remote` and `live-test-remote` build there) are
cleaned the same way: when a command ends, the host's agent runs the cleaner on its target directory
(its text goes with the job), detached at nice 19, at most once every 15 minutes per directory, and
logs it in that directory's `.cleaned.log`. A profile directory another command builds in is skipped
and cleaned when that command ends. Its only Relax home is the farm's own, under the root. Before
farm 17 nothing did this: one host's target directories reached 207 GB, 195 GB of it leftovers, and
the host fell under `min_free_gb` (2026-11-16).

The root is pruned too, after a command ends, at most hourly, logged in `root.log` beside it: what
no run has read or written for 3 days goes (a directory none of whose files was used goes whole; in
a directory still in use, only the copies the mirror sent, which it sends again when a run needs
them; a file a command wrote stays while its directory is in use). While the host has less than
`min_free_gb` + 40 GB free, the same goes at 1 day idle, then 6 hours. The farm's Relax home is left
to the cleaner. A compile that misses a pruned copy fails once, runs on the build machine, and the
host is asked what it holds. Before farm 18 the root kept every copy and run output (95 GB on one
host). A file the mirror is asked about for a run counts as used from then on, each path is checked
again just before it goes (the walk's view of a big tree is minutes old by then), and a directory a
command runs in is never pruned (`busy.noindex` holds a lock per directory while it runs): before
farm 22 a pass removed a checkout's `Cargo.lock` and `rust-toolchain.toml` under a run that had just
been told they were there, and that run's cargo resolved its crates afresh on the host's default
toolchain. The commands' `TMPDIR` is the farm's (`tmp.noindex`), pruned the same way (every file
counts, not only the sent ones), so what their runs leave is the farm's to free.

#### A host short of disk

A host is never left out quietly. Under `min_free_gb` free, the probe that sees it starts
`farm-agent free` there (at most every 10 minutes): test targets no command runs in go, least
recently used first, then the rest are cleaned, then the root and `TMPDIR` go down to what the last
hour used, until `min_free_gb` + 40 GB are free. It logs the result in
`~/Library/Caches/rerail-farm/freeing/<host>.json` here, and when the host is still short the
farm shows a notification on this machine's screen (at most every 6 hours a host): how much it
freed, how much it still holds there, and that the rest is the host's own data. `status` and
`test-remote` say it too. The host stays in use down to `floor_free_gb` (20 GB, left to the host's
own work); under that it is out until the next probe finds room. Before farm 22 a host under 40 GB
free was simply skipped, while 1.8 TB of its own data, not the farm's 18 GB, had filled it.

## Told at the build

What an agent does wrong is printed in its own build output, not left for someone to find. At the
first compile of each cargo command the wrapper prints, as `warning: rerail-farm:` lines: a nice
value under 19, a rustc other than the checkout pins, a development build of the engine's CLI without
`--features test-hooks` (it compiles every dependency crate apart from the tests' build of it:
native/Cargo.toml), a workspace crate compiled with other features than a build of the same target
and profile in the last 2 hours did (a second variant, compiled at each edit with every crate above
it and with its own incremental state; said once per command and difference, with the crates, the
features and the earlier command, and repeated at the checkout's builds for 3 hours; the records are
in `~/Library/Caches/rerail-farm/variants/`), and the checkout's latest `diagnose --brief`
findings (another toolchain or cargo configuration than the active worktrees, profile settings that
differ, scripts that set the variables builds share), each with what to do. A cargo command that
runs tests here (`cargo test` or `cargo nextest run`, filtered or not) while a test host has a free
test slot is told the `test-remote` command that runs it there (`RERAIL_FARM_TESTS_HERE=1` when it must run here). After a cleaner run, at most hourly per checkout, the job writes
those findings (`~/Library/Caches/rerail-farm/findings/<key>`). A build that
turns the wrapper off (`RUSTC_WRAPPER=`) is stopped by `native/build.rs` instead, with the fix in
its message.
A variable builds must share (CC, CXX, SOURCE_DATE_EPOCH, JEMALLOC_OVERRIDE) stops the
build at the first compile. `RERAIL_FARM_QUIET=1` silences the warnings.

## Configuration

The farm's configuration is local to this Mac and never in git: `$RERAIL_FARM_CONFIG` if set, else
`~/.config/rerail-farm/farm.json` (`farm.example.json` shows its shape). Without one there are no
hosts, and everything compiles and tests here, as without the farm.

| Key | Meaning |
|---|---|
| `hosts` | the hosts: `{"name", "cores", "enabled", "tests", "link", "unattended", "os", "nice", "max_target_gb"}` |
| `os` | `"macos"` (default) or `"linux"`: a Linux host runs test commands only (below, "Linux hosts") |
| `nice` | the nice value of `test-remote`'s commands there (default 10; 0 where the farm is an equal of the host's other work) |
| `fleet` | `true` on a nera-fleet node: while any fleet job runs there (a container labelled `fleet.job`), the farm starts no command there, and asks again each minute. The two are equals: neither preempts the other, and what runs finishes |
| `name` | what ssh connects to (a `Host` entry of `~/.ssh/config`) |
| `enabled` | `false` leaves a host out (default `true`) |
| `tests` | whether `scripts/dev/test-remote` runs there (default `true`) |
| `test_slots` | how many `test-remote` commands the host runs at once (default: a third of its cores). One at a time left a 16-core host mostly idle while commands queued for 25 minutes; a command waits only when every slot of every host is taken, and goes to whichever frees first. A checkout's second command goes to another host when one has a slot (its commands on one host share a target directory, which cargo builds in one at a time) |
| `link` | the host's network interface the farm's traffic must arrive on (its fast link); unset: any |
| `min_free_gb` | under this much free disk, the farm frees its own data on the host and says so (default 40) |
| `floor_free_gb` | the free disk left to the host's own work: under it the host is not used (default 20) |
| `max_target_gb` | the most a host's test target directories hold together (default 150 GB; per host too) |
| `max_rtt_ms` | the slowest round trip a host may have (default 150), for a host without `link` |
| `min_mbps` | the least throughput to it, MB/s (default 8), for a host without `link`: with one, the link check catches a fall back to WiFi, and both are only reported. Both are measured through ssh: the round trip is the least of three (each starts an ssh process, which a loaded Mac delays), and the throughput is 32 MB written through a plain pipe (Python's `run(input=)` writes 512 bytes at a time and read 57 MB/s on a Thunderbolt link that does 270 through ssh and 800 raw). They still fall when either Mac is busy: a low figure is no proof of a slow link |
| `tcp` | `false` keeps the live mirror's data on ssh (default `true`; also `RERAIL_FARM_TCP=0`) |

A host takes a compile per core while nobody has touched its keyboard or mouse for 5 minutes, and
half as many otherwise (`HIDIdleTime`). A host nobody sits at whose
trackpad reports touches on its own (one did, every 0.1–4 s, on 2026-11-15) is marked
`"unattended": true`: then its input counts for nothing. Compiles take slots in turn across the hosts (each host's
first slot, then each one's second), so every host works, not only the first listed. Not its load average: macOS's background analysis (Photos,
Spotlight) keeps that high on an idle Mac, at a priority every compile preempts.

### Linux hosts

A Linux host (`"os": "linux"`) runs `test-remote` commands, as a Linux build and run of the
checkout: the engine's Linux build is CI's, on Ubuntu 24.04. It takes nothing else. A compile of
this Mac's cannot run there: a crate's proc-macros are macOS libraries its rustc cannot load,
and a build script's C needs the macOS SDK. Nor does a Relax run (`live-test-remote`).

`test-remote` gives a command to a Mac first, and to a Linux host when every Mac slot is taken.
A command that names Metal, or sends a `--tree` (a program built here is a macOS one), runs on a
Mac only; `--os` chooses (`macos`, `linux`, `any`). The host's line says it is Linux, so a result
is never taken for a macOS one.

There the farm's directory is `~/.cache/rerail-farm` (XDG's), the mirror's attribute is
`user.org.rerail.farm.sent`, free disk is statvfs's, and the probe reads the arrival interface
with `ip`, the load from `/proc`, NFS mounts with `findmnt`. A server has no keyboard: it is never
attended. `setup` installs the pinned toolchain with the host's rustup; rustup itself and
`cargo-nextest` (the version here) are installed once by hand, in the host user's home:

    curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --default-toolchain none --profile minimal
    curl -LsSf https://get.nexte.st/<version>/linux-arm | tar zxf - -C ~/.cargo/bin

### The farm's root on a host

A host is someone's own Mac, with its own work. The farm writes only under
`~/Library/Caches/rerail-farm` there, never anywhere else: the mirrored files in `root.noindex`,
where this machine's path P is kept at `root.noindex` + P, and `test-remote`'s target directories
in `targets.noindex`. Nothing of the host's own is moved, renamed, written or mounted over. (An
earlier version kept the files at this machine's own paths, and moved a host's own `~/orca`
aside to make room: everything on that host that used its `~/orca` broke until it was put back.)

A compile must give the same bytes there as here: its output goes into the shared cache, and every
crate above it keys on those bytes. rustc hashes each source file's real path into its output,
remapped or not (measured: the same crate compiled from two directories, each remapped to the same
path, gives other bytes; the output directory makes no difference). So a remote compile reads its
sources at the path they have here, which only cargo's own downloads allow: a crate of
`~/.cargo/registry/src` (or `git/checkouts`) is the same files at the same path on every machine.
The daemon checks the host's copy file for file (sha256, once a crate per connection) and the
compile reads it in place; nothing is written there. A crate the host lacks, or has otherwise,
compiles here, as do the workspace's own crates and a crate whose build script wrote `OUT_DIR`.

What it does need from here, its dependencies' `.rmeta` files and native libraries, is kept under
the root, and the agent maps those paths (`farm-agent`, `run`): each is replaced by where the host
keeps it, in rustc's arguments, its environment and its working
directory, and `--remap-path-prefix <root>=/` goes first among the arguments: rustc applies the last
remap that matches, so every remap the compile had keeps its meaning, and paths that none
reached lose the root. The dep-info and rustc's messages have the root taken out. An output that
still names the root (a path that no remap reaches: `env!("CARGO_MANIFEST_DIR")` written into the
crate) is not used: the compile runs here, and that crate compiles here from then on.

A file the mirror writes there has the time of the write as its mtime, not this machine's: a
cargo on the host (`test-remote`) then rebuilds what a file changed, however late the file
arrived and however old its mtime here. The file keeps this machine's mtime and size in an
extended attribute, and the mirror compares those.

### The hosts' backups and indexing

`~/Library/Caches` is not backed up by default; `host-privacy` (run by `setup`) excludes the farm's
directories from Time Machine explicitly too. Spotlight skips folders whose names end in
`.noindex`: the root and the target directories. `host-privacy --spotlight` (sudo, from a terminal)
also puts them in Spotlight's privacy list. Its report counts what Spotlight still has indexed.

## Health

Hosts come and go: they sleep, leave the network, fall back to WiFi, fill their disk, or have their
copies cleaned. Nothing waits on one for long, and anything a host cannot do runs here.

- **Probe.** Before a host is used its health is read: one ssh command (3 s to connect) reports the
  interface the connection arrived on, its free disk, its idle time and its mounts; the round
  trip is timed, and every 10 minutes (or while it is slow) a 4 MB transfer measures the
  throughput. A host short of disk or past the limits is not used.
- **Blocked here, not down.** macOS's Local Network privacy can deny the application the agents
  run under (and every program it starts: ssh, nc, ping) any connection to a local address, as
  "No route to host", while the hosts answer everyone else; it happened to every host at once on
  2026-11-16. When a host cannot be reached, the probe tries it again from a job launchd starts:
  if that reaches it, the reason names the application and the setting (System Settings, Privacy
  & Security, Local Network), and the host is probed again every 30 s rather than backed off.
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
- **Lost mid-job.** A compile whose send or run fails there runs here, and the host is marked down
  when its connection or agent is gone, or a file could not be written there (its disk full:
  agent code 70; the daemon then drops the agent and forgets what it sent). A test run whose ssh
  connection drops (exit 255) runs again here.
- **Cleaned copies.** The mirror remembers what it sent each host, and a host restarted or cleaned
  forgets: an agent is asked what it holds when it starts, a compile that read a stale file clears
  what was sent, and `test-remote` asks the host again for every file of the checkout before it runs.
  Nothing but the farm writes under the root, so nothing else makes what the mirror remembers wrong.
- **Disk.** `test-remote`'s target directories (one per checkout, by a hash of its path) are
  cleaned after each command (above), go after 3 unused days, and the least recently used go while
  the host has less than `min_free_gb` + 40 GB free: pruning only to `min_free_gb` left the host
  just above it, and the next build took it under. Never one a command is running in: each command
  holds its target's lock (`targets.noindex/.locks`) until it ends, and pruning skips a held one
  (farm 21; before, a checkout's run on a short disk removed another's target mid-build).
  The targets together stay under `max_target_gb` (150 GB): over it, the least recently used no
  command holds go. While a command runs, its agent checks the host every minute (one agent at a
  time): targets due a clean are cleaned whether or not their own commands ever ended, the budget is
  measured every 10 minutes, and under `min_free_gb` + 40 GB free the least recently used targets
  and the root's idle copies go. A target goes by a rename aside first, so a removal cut short
  leaves no half a target. An agent whose connection drops (its caller killed) stays to stop the
  command and clean after it. Before farm 27 a host's targets were checked only when a command
  started and cleaned only when one ended normally: 16 checkouts' targets reached 318 GB, 33 GB of
  one of them leftovers never cleaned, and concurrent builds took the host from 255 GB free to none
  within the hour (2026-11-17).

`scripts/build/farm/status` probes every host now and prints whether it is usable and why not,
with its round trip, throughput and free disk (`--json` for scripts; exit 1 when none is usable).

## No benchmark leases

Farm 9 to 19 had `scripts/dev/quiet-run`: a lease on a machine for a benchmark, with fair turns, a
drain of the farm's work, a 10-minute cap and warnings to checkouts that loaded a leased machine.
Removed in farm 20 (2026-11-16): the turns, drains and refusals stalled every agent's work, and
nothing waits on a lease any more. `quiet-run` now runs its COMMAND at once, through `test-remote`
(or here with `--here`), so existing scripts still work. A benchmark is an ordinary command; its
result says how loaded the machine was.

## No shared volume, no copy mode

Two other ways were measured and removed (docs/final-shape/notes/t37, §4). Copy mode (tar over ssh
per compile) cost as much as the compile itself. A shared volume (this machine's checkouts over
NFS) was slower than the mirror on every run (105–185 s against 91 s for the same crates, every
compile forced to miss), and lost compiles to stale views (E0463, E0460), since the NFS server
commits each write before it answers; and it needed the hosts' system configuration changed
(an automount over the host's own path). A setting that answers before committing
(`vfs.generic.nfs.server.async`) was not measured.

## Live mirror

A host keeps a copy of what its compiles read on its own disk, under its farm root, and is sent
only what changed, over one connection that stays open. A daemon here
(`mirror.py`, started by the first compile, gone after 15 idle minutes) holds a `farm-agent` per
host over ssh. For each compile it scans what the compile may read from here (the dependency
directory and native library directories; `test-remote` and `live-test-remote` send a checkout's
files the same way; one scan serves every compile that started before it), sends each file whose size or mtime differs from what it last sent, then the
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
on ssh. Every file the compile read (its dep-info, and each `--extern` crate, which the dep-info leaves
out and the host is asked for again before each compile) must have the size and mtime
it has here, or the compile runs here. The agent refuses to write when its root is under a network
mount.

`RERAIL_FARM_LOG=file` logs each decision (hot, hit: nothing logged, here, or which host) with
where a remote compile's time went, and why a compile did not go remote. `RERAIL_FARM_LOCAL=1`
compiles every miss here.
