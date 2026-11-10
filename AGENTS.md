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
