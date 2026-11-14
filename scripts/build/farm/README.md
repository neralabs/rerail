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
- **Spreading.** On a miss, a library crate compiles on another Mac when one has a free core
  (the hosts in `~/.config/rerail-farm/hosts.json`, reached by ssh). The sources, the crate's dependencies' `.rmeta` files and
  its `OUT_DIR` are copied to the same absolute paths there, only what changed since the last copy,
  and the outputs are copied back.
- **Editing.** A workspace crate that missed in the last 2 hours is being edited here. It skips the
  cache and compiles incrementally on this Mac, as without the farm.

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
| `farm.py` | both of them |
| `remote-rustc` | runs one compile on a remote Mac |
| `setup` | installs all of it, and `remote-rustc` on each host |
| `hosts.example.json` | the hosts, copied to `~/.config/rerail-farm/hosts.json` |

## Setup

```
scripts/build/farm/setup path/to/patched/sccache
```

The sccache is upstream main with PRs #3021 (`SCCACHE_BASEDIRS_REMAP_PATHS`) and #3022
(`SCCACHE_BASEDIRS_VERBATIM_ENV_DEPS`) merged in. Each host needs ssh without a password (its name in
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
```

`JEMALLOC_OVERRIDE` set to a prebuilt `libjemalloc_pic.a` saves `tikv-jemalloc-sys`'s 3-minute
configure-and-make in every new target directory; its build script is never cached.

## Hosts

`hosts.json` lists `{"name", "cores", "enabled"}`. A host takes as many compiles as it has cores
less its load average (read at most every 20 s), so a machine in use takes fewer. A host behind a
slow link is better disabled: the first copy of a checkout (about 350 MB) to it is slow.

`RERAIL_FARM_LOG=file` logs each decision: hot, hit (nothing logged), here, or which host and how long.
