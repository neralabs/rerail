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

None. What was open here, closed on 2026-11-16:

- **The tests that parse rule files**: the hub moved them, and 54 more, into the engine
  (`tools/cratify/runtime_hosts.py`, `runtime-hosts.tsv`), and their ignores are gone.
  `prepared::identity_tests` then failed on a stale assertion: outputs became rows (0239), and
  the test read them as text. With that fixed, the first run after a rebuild still failed once
  (7 of 8 passed). The cause: the test store's template is filled by whichever test process asks
  first, and that process kept the template's phase-0 include graphs for itself. Its own runs then
  derived none, and prepared one program fewer than a process that found the template made. The
  template's graphs are no longer kept for the process (`test_store::filling`, `phase0::graphs`).
- **The decision-0104 lints** run again. Of the maps added while they did not run, eight are
  state (`SHAPE`, `SHIPPED`, `READ_BEFORE`, `OWN`, `COMMAND`, `DECIDING`, `RUN_PHASES`, `ENTERED`).
  Four listed entries were stale and are cut. `CROSS_FILE` (a registration at load), `RECORDED`
  (test placements under a test's guard) and `CENSUS` (a test module's read of the shipped files)
  stay state: none is a result a run reads.
- **`aggregate_demand_executes_only_requested_groups_and_keeps_empty_groups`** passes again: the
  6-row step was a sibling test's seed (`rr_agg2`) in the process's shared profile. The hub's
  filter on the helper's own demand now leaves it out.

Found while closing them: the corpus tests' module, run in parallel, ran past 115 s. A store's
`frames` upkeep job outlived the store. Its cached connection read the deleted file, so the job
stayed due, and its step made a new store at the path. Each `finish_soon` then ran a hundred
passes over every gone store. Fixed in `content_store::upkeep::Frames`; the module takes 5 s.
