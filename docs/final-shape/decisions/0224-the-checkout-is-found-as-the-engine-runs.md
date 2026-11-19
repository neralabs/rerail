# 0224: The checkout is found as the engine runs, not written into its build

Status: built (2026-11-14).

## Intro

**The problem.** Three of the engine's library crates wrote their checkout's directory into what they
compiled, through `env!("CARGO_MANIFEST_DIR")`:

- `rerail-store`: the compression dictionaries, by `include_bytes!` of a path built from it;
- `rerail-lang`: each dialect's `grammar.json`, by `include_str!` the same way;
- `rerail-logic`: where it reads its `.rr` files as it runs: the builtin typing programs (`phases`,
  `bridge`) and the rule library (`rr::library`, when `RERAIL_RR_LIB` is not set).

So each of these crates was a different crate in every checkout, and so was every crate that depends on
them, which is most of the engine. A compiler cache that the worktrees share (t37 §4) could reuse none of
it. It has to hash such a crate's path as it is: reused in another checkout, the crate would read that
checkout's files.

**The decision.**

- A file the build reads is named relative to the file that reads it (`../../dictionaries/…`,
  `../../../grammars/…`). It is the same file, and the crate no longer names a checkout.
- A file the engine reads as it runs is found as it runs: `source_tree::root()` (in `rerail-store`). It
  is the first directory holding `native/Cargo.toml` and `docs/rule-syntax` found:
  1. at `RERAIL_SOURCE_ROOT`;
  2. above the file the code was loaded from (the program, a test binary or the Node addon, each built
     into its checkout's `target/`);
  3. above the package `cargo run` or `cargo test` runs (`CARGO_MANIFEST_DIR`, read as the program
     runs);
  4. above the working directory.

  Outside a checkout there is none, as before: an installed engine's build directory did not exist on
  its machine either.

## Checked

- With the cwd inside and outside the checkout, the CLI's findings on the smoke corpus are the same.
- `source_tree`'s test, and the typing phase, phases and bridge tests (8), pass.
- The CLI builds, and the wasm build checks.
- Of the workspace's crates, only these three listed `CARGO_MANIFEST_DIR` in their dep-info (the
  variables rustc records a crate read). Tests still read it; a test build is a checkout's own.
