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

Each vitest run, here and in `integration/`, points the temp folder (`TMPDIR`, `TEMP`, `TMP`) at a folder of its own,
`rerail-vitest-<pid>-*`, and removes it when the run ends, so a test needs no cleanup to leave nothing behind. A run
killed before its end leaves the folder, and the next run removes it once that pid has ended
([`support/temporary/run-folder.ts`](support/temporary/run-folder.ts)). On macOS the folder goes in `/tmp`, since the
per-user temp folder leaves no room for a daemon's socket path below it. On Linux, keep a `TMPDIR` you set to 24 bytes
or fewer: the deepest socket a test makes ends 54 bytes below the temp folder, the run folder adds up to 29, and a
socket path holds at most 107.
