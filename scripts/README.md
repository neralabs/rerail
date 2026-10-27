# scripts

Repository scripts, grouped by what they are for. Their tests live in `test/scripts/`.

- `build/`: steps of `npm run build` and `npm pack` (copy the UI, require cargo, repair SQLite, stamp plugin
  versions).
- `check/`: the checks `npm run check` and `check:*` run, including the layer boundary lint
  (`check-boundaries.mjs`), the Windows checkout check (`check-windows-paths.mjs`: every tracked path short
  enough and named so Windows can check it out), the internal reference check (`check-internal-refs.mjs`: no
  internal ticket ID in a tracked path or text) and the licence allowlist.
- `deps/`: pins that follow a dependency's version (`sync-pins.mjs`, `npm run deps:sync`): the OMP host and its release
  digests and the playground's SDK follow the OMP SDK, the ast-grep differential test's reference follows the ast-grep
  CLI. `.github/workflows/dependabot-pins.yml` runs it on Dependabot's pull requests; `npm run check` fails on drift.
- `dev/`: a development install from this checkout (`just rerail-dev`), its restore, nuke and smoke runs, and the
  dashboard scenario launcher for design review (`ui-scenario.ts`, `just ui-scenario <scenario>`).
- `dsl/`: the code-rule DSL benchmark.
- `generate/`: generators of committed engine and host data and the DSL reference page (manifest: `generate/README.md`).
- `host/`: real-host runs: OMP install, host smoke, lifecycle and auth checks, the Windows install smoke.
- `pack-data/`: the derivation of the packs' derived test data from their pins (`derive.ts`, `npm run fixtures:derive`),
  with one recipe per pack in `recipes/`, the runtimes it installs (`runtimes/`, `runtimes.ts`), the ESLint
  expected-findings helper the ESLint-based generators share (`eslint-expected.mjs`) and the renamer that writes the
  ECMA rename variants (`rename/`); a maintainer and the bump workflow run it to make the data, never pull request CI
  (CONTRIBUTING.md, "Pack test data"; docs/upstream.md).
- `release/`: the release. The Release pull request (`.github/workflows/release-pr.yml`): every package's version,
  core's when what it ships changed, and core's tag on merge. The rule packs' release
  (`.github/workflows/release-packs.yml`): which packs to publish, the publish gate and packing, publishing under
  `latest`, putting a pack back, and the lockfile sync after `changeset version`. The dist-tag check after a publish.
- `review/`: the guidance review (`just rerail-guidance-review`): every rule pack rule run on its examples and upstream
  fixtures, with the steer the coding agent reads, in a local page where each rule's guidance is approved or sent back
  with comments.
- `samples/`: the packs' real-code sample sets (`format.ts` says what one holds): the sample model and archive
  (`samples.ts`, `pack.ts`), the clean-sample relevance selection (`relevance.ts`), the ESLint test set's conversion
  (`convert-eslint-test-set.ts`), and the sources' licences with the `LICENSES.md` and `SAMPLES.md` they write
  (`licences.ts`).
- `s3/`: the packs' test data archives in S3 (`pack-fixtures.mjs`: `npm run fixtures:pull`, `fixtures:stage` and
  `fixtures:push`, and the `pack`, `upload` and `confirm` steps of `.github/workflows/pack-data-publish.yml`; the
  canonical tar and `test-data.lock` in `test-data-archive.mjs`; CONTRIBUTING.md, "Pack test data"), with the messages
  its consumers give in Python (`pack_fixtures.py`).
- `upstream/`: the upstream release check (`releases.ts`, `npm run upstream:releases`) and the pin bump (`bump.ts`,
  `npm run upstream:bump`) the upstream workflows run (docs/upstream.md).
