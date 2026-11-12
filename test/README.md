# test

Unit tests, run by `npm test` (vitest over this directory). They mirror the layers of [`src/`](../src/README.md):
`test/engine/` tests `src/engine/`, `test/hosts/<harness>/` tests `src/hosts/<harness>/`, and so on. A test under
`test/<layer>/` may import its own layer's modules; anything else in `src/` it reaches through that layer's `index.ts`
(`npm run check:boundaries`).

- `support/`: shared test helpers (oracle paths, hook events, a native test daemon, the per-file Rerail home, `git.ts`
  for repositories with a fixed identity and none of the machine's Git configuration, and Merge Assist's stand-ins in
  `merge-assist/`: `workspace.ts`, real repositories and a database with a fake `gh`, and `status.ts`, sample statuses
  and a recording service). A layer's own helpers live in `test/<layer>/support/` (`runtime/merge-assist/support/`
  holds the files and session runtime the conflict tests share); a helper a second layer needs moves here.
- `scripts/`: tests of the repository scripts under `scripts/`; `ensure-sqlite.test.mjs` runs under `node --test`
  (`npm run test:scripts`), as does `docs/test_transducer_design.py`, which checks `docs/transducer-design.md`.
- `ui/`: the dashboard page (`ui/`): its view model, and its views rendered without a browser into the stand-in DOM of
  `ui/support/fake-dom.ts`, which parses no markup, so a test tells text that stayed text from markup a view built;
  `ui/support/merge-page.ts` mounts the Merge Assist page on it with its requests held for the test to answer.
- `fixtures/`: test data.

Tests that need a built package and real hosts are in [`integration/`](../integration/README.md).
