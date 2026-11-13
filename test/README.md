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
`rv-<pid>-*`, and removes it when the run ends
([`support/temporary/run-folder.ts`](support/temporary/run-folder.ts)). So a folder a test leaves goes when the run
ends; a test still removes what a later test in the same run could see, and what it makes outside `tmpdir()`. A run
killed before its end leaves the folder, and a later run removes it once that run has ended. The folder's `.owner`
file says where its pid means something (the host and, on Linux, the pid namespace), and the run touches it every
minute. So a run from the same place goes once its pid has ended, one from another container or host that shares the
temp folder once it has stopped touching the file for ten minutes, and any run a day after it stopped. The owner file
also names the folder vitest copies the run's modules into when its module cache is off or cannot be made: vitest
makes that folder in the temp folder before the run folder exists, and it goes with the run folder, unless the run
was killed after its teardown removed the run folder, while vitest closed.

On Linux and macOS the folder goes in `/tmp`, or in the folder `RERAIL_TEST_SHORT_TEMP` names, when the temp folder
leaves no room below it for the deepest socket a test binds: that socket ends 54 bytes below the run folder, the run
folder adds up to 18 bytes with its separator, and a socket path holds at most 107 bytes on Linux and 103 on macOS.
macOS's per-user temp folder never leaves the room. On Linux a `TMPDIR` of 35 bytes or fewer always does, since a
pid has at most 7 digits; a longer one does while its bytes and the pid's digits come to 42 or fewer. `/tmp` can be
another filesystem than the temp folder (tmpfs on many Linux hosts), and a test that depends on the order a folder
lists its files in can pass in one and fail in the other. When that folder refuses the run folder (a read-only `/tmp`
in a sandbox), the run says so on stderr and goes on without one: its tests make their folders in the temp folder, as
before there was a run folder.
