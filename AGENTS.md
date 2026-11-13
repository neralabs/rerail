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

| Target | Measured (2026-11-10) |
|---|---|
| The CLI alone (`--bin rerail-engine`), after a one-file edit | 135 s wall, 629 s CPU |
| The library's tests (`--lib --tests`), after a change in `accel` | 380 s wall |

So a run of the CLI builds `--bin rerail-engine` only, and the test target is built when
tests are run, not with every build.

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
