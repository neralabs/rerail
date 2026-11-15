# 0128: Every crate's test binary builds and runs its own tests

Status: built (2026-11-12).

## Intro

**The problem.** After the crate split, the test binaries of rerail-logic and
rerail-device-metal did not build. So no binary compiled some tests, including retain's frame
tests, the session tests and the corpus tests, and they had not run since the split. The causes:

- the test hooks of the crates below were not switched on in a crate's own test build;
- some tests used helpers private to the engine (`prefilter::tests::temp`, `open`);
- inline test modules in production files read code above their crate (`tools`, `waves`,
  `sql_executor`, Metal `transducer`);
- the vendored grammars' C was compiled only by the engine's build script;
- the build identities and the store's SQL hazards were set only as the engine loaded.

**The decision.** Each crate's test binary builds and runs its tests by itself.

## How

- **Test hooks.** A crate's `[dev-dependencies]` repeat its crate dependencies with
  `features = ["test-hooks"]` (and `tempfile`), so its own test build has them.
- **Shared helpers.** Tests take `fixtures::temp()` and `ContentStore::open(..)` directly; the
  engine's `prefilter::tests` only re-exported them.
- **Tests that read code above their crate** are moved out of their production file into a file
  of their own (`session/tests.rs`, `matchers/tests.rs`, `corpus/tests.rs`, `lab/tests.rs`,
  `base_tests.rs`). The crate `tools/cratify/test-hosts.tsv` names (here the engine) mounts each
  one in its facade, as `#[cfg(test)] pub mod tests;`.
- **Grammars.** rerail-lang's build script compiles the vendored grammars
  (`src/build_grammars.rs`), as the crate whose code calls them, so every crate that reaches
  `GrammarDialect::ts_language` links them. The engine's build script reads `GRAMMARS` only for
  its identities.
- **Build identities.** A test build (`test-hooks`) that no engine has set reads fixed
  identities (`build_ids::UNSET`). Any other build without them panics, as before.
- **The device.** rerail-device-metal's own test binary registers Metal as it loads, as the
  engine does (`install_devices`).
- **SQL hazards.** The store reports its statements and waits to the profile directly
  (`rerail_prof`, which it already depends on), not through a hook the engine set as it loaded.
  So any crate's test that profiles sees them.

Tests that need the language registry (languages register as the engine loads) are hosted by the
engine too: `typing::phase_tests`, `waves::owner::tests`, `logic::liveness::tests` and
`strings::registered_tests`.

## Checked

- rerail-device-metal builds, and every test not listed under Open passes, the 16 device
  tests among them.
- rerail-logic builds; retain's frame tests (7) pass, and so does every test not listed under Open.
- In the engine, the moved modules pass: session (10), matchers (7), corpus (12), base (38).

## Open

Each of these is ignored with its reason (`open (decision 0128)`). None had run since the split.

- **17 tests that parse rule files** in `logic::plan` (`phases`, `prepared`, `reuse`,
  `cache_tests`, `phase0`, `compose`, `native_demand`, `prepared::identity_tests`). Only the
  engine registers languages (`install_languages` reaches embedders, residency, the workspace,
  eval and resolution), so these are the engine's to host. Many are inline modules in production
  files: each must be moved out to a file, as above, and the private items it reads widened.
  Four more in rerail-device-metal, for the same reason: the launch budget test, the slab
  admission test, the resident tape test and the T-program split test.
- **The decision-0104 lints** (`facts_only`: `every_table_is_facts_or_listed`,
  `every_kept_map_is_state_or_listed`). Since they last ran, 12 tables (mostly the device tests'
  scratch tables) and 19 maps have been added. Each needs a judgement: state, scratch, or a
  result that should be facts (`CROSS_FILE`, `CENSUS` and `RECORDED` may be).
- **`aggregate_demand_executes_only_requested_groups_and_keeps_empty_groups`**: a seed step of
  `rr_agg2` makes 6 rows for 3 demanded groups (`1:3,3:6,6:3,3:3` in its trace), where the test
  allows at most 3.
