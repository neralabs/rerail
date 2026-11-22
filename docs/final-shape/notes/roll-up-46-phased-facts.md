# Roll-up 46: final-rerail-shape-phased-facts (1c87fb5754) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 0c09561b23 (roll-up 45, Q466). Theirs: final-rerail-shape-phased-facts at
1c87fb5754. Its merge base is 12a217061f. Theirs brings 0494 step 7, done by 4265ccd1d0 through 1c87fb5754:

- a 4-byte column may be dictionary-coded, or hold its values' low bits under key runs (`rows::ColDir::runs`), in
  every read (`ColDir::get`, CLIF's `col_get`/`col_high`, the kernels' `relation.h`);
- an axis's solve takes 8 bytes a row;
- a batch's axes over one set share their solves (`within::Solves`), and an `inside` that nests nowhere reads its
  `nearest`'s relation.

## Items

1. **`within.rs`: theirs' `solve`, with ours' Windows gate.** Theirs restructured the axis solve into `Solves` and
   `solve(nearest)`. Ours' only change in that hunk was `windows` in the device gate (0440). The merged `solve` has
   the gate with `windows`, and every device gate in the file names it.
2. **`device_metal/Cargo.toml`: one `rerail-clif` dev-dependency.** Both sides added it, for the same reader
   (`rerail_clif::view::cache`, theirs' `text_find` among it). `text_find` is not gated on test-hooks, and ours'
   features already pass `rerail-clif/test-hooks`. So ours' plain entry stays, and its comment names both readers.
3. **`rr_aot/Cargo.toml` is ours'.** It already has `rerail-tape-format` as a dependency and in test-hooks, which is
   what theirs added, among the crate split's crates.
4. **Decision 0494: theirs' side.** Theirs holds step 6's census, step 7, and the same "Reads side by side"
   section that ours added (word for word). Theirs' side keeps the steps in order with that section once.

## Checks

- spark2: the clif, ukl and blocks libraries pass, 101/101, including the generated `relation.h`'s check. The
  syntax, within, rows and setfix device tests on the CPU pass, 43/43. Clippy over the touched crates is clean.
- m1max, compile and clang only: the ukl header tests pass, 10/10.
