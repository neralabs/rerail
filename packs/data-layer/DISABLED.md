# The data-layer packs are disabled

Since 2026-11-23 the data-layer packs are not run: not by the engine's tests, not by CI.

- The pack's manifest (`pack.json`) activates no rules, as before: product runs never ran it.
- Every test that reads its rules or its test data is ignored with the reason
  `data-layer packs disabled (packs/data-layer/DISABLED.md)`. A test that was already ignored keeps its old reason
  after that one. `rg 'data-layer packs disabled'` lists them.
- `pack_test_data` (`native/src/code_rule_dsl/test_support.rs`) refuses a path under `packs/data-layer/`, so such a
  test run by name with `--ignored` stops and says why.
- CI no longer fetches the pack's test data (`.github/workflows/ci.yml`).

Why: the pack's rules, in its pulled test data, read a `tape_line` relation that no converter declares. Their
conversion and replay tests fail on that, not on any change to the engine.

The engine's own data-layer rule library (`lib/data-layer/`) and its in-tree programs
(`native/src/code_rule_dsl/logic/programs/data_layer/`) are not this pack, and their tests still run.

To enable the pack again, reverse each point above and remove this file.
