# test

Unit tests, run by `npm test` (vitest over this directory). They mirror the layers of [`src/`](../src/README.md):
`test/engine/` tests `src/engine/`, `test/hosts/<harness>/` tests `src/hosts/<harness>/`, and so on. A test under
`test/<layer>/` may import its own layer's modules; anything else in `src/` it reaches through that layer's `index.ts`
(`npm run check:boundaries`).

- `support/`: shared test helpers (oracle paths, hook events, a native test daemon, the per-file Rerail home). A
  layer's own helpers live in `test/<layer>/support/`.
- `scripts/`: tests of the repository scripts under `scripts/`; `ensure-sqlite.test.mjs` runs under `node --test`
  (`npm run test:scripts`), as does `docs/test_transducer_design.py`, which checks `docs/transducer-design.md`.
- `ui/`: the dashboard page's view model (`ui/`).
- `fixtures/`: test data.

Tests that need a built package and real hosts are in [`integration/`](../integration/README.md).
