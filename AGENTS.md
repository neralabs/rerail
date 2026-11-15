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

### Test tiers (cargo-nextest)

`.config/nextest.toml` splits the library's tests in two tiers (docs/final-shape/notes/t37, §2).
From `native/`:

```
nice -n 19 cargo nextest run --lib -E 'test(/some::filter/)'   # targeted: the usual run
nice -n 19 cargo nextest run --lib                             # the quick tier: all but the slow tests
nice -n 19 cargo nextest run --lib --profile gate              # everything: a phase gate only
nice -n 19 cargo nextest run --lib --ignore-default-filter NAME  # one slow test by name
```

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
scripts/dev/test-remote -- cargo nextest run --lib --profile gate
scripts/dev/test-remote -- cargo nextest run --lib -E 'test(/some::filter/)'
scripts/dev/live-test-remote PACKAGE --lib -- FILTER   # Relax's live loop on a host
```

`test-remote` sends this checkout's changes to a usable host and runs the command there, from
`native/` (or `--dir`), in that host's own warm target directory. A host can be asleep, gone,
on a slow link, or short of disk, or its copy can have been cleaned. The scripts check for this
themselves: a host that fails a probe is skipped, with a growing back-off, and a lost
connection reruns the command here at `nice -n 19`. Don't work around a host that is down. Run
`status` to see why, and go on locally. `--remote-only` exits 69 when no host can take the
command.

The farm's compiler wrapper is opt-in: the global cargo configuration uses plain sccache.
`RUSTC_WRAPPER=~/.local/share/rerail-farm/rustc-wrapper` (an absolute path) sends a build's
dependency misses to the hosts. `RERAIL_FARM_LOCAL=1` keeps every compile here, and
`RERAIL_FARM_LOG=FILE` logs where each compile ran.

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
