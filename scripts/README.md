# scripts

Repository scripts, grouped by what they are for. Their tests live in `test/scripts/`.

- `build/`: steps of `npm run build` and `npm pack` (copy the UI, require cargo, repair SQLite, stamp plugin
  versions).
- `check/`: the checks `npm run check` and `check:*` run, including the layer boundary lint
  (`check-boundaries.mjs`), the Windows checkout check (`check-windows-paths.mjs`: every tracked path short
  enough and named so Windows can check it out), the internal reference check (`check-internal-refs.mjs`: no
  internal ticket ID in a tracked path or text) and the licence allowlist.
- `deps/`: pins that follow a dependency's version (`sync-pins.mjs`, `npm run deps:sync`): the OMP host and its release
  digests follow the OMP SDK, the ast-grep differential test's reference follows the ast-grep CLI.
  `.github/workflows/dependabot-pins.yml` runs it on Dependabot's pull requests; `npm run check` fails on drift.
- `dev/`: a development install from this checkout (`just rerail-dev`), its restore, nuke and smoke runs, and the
  dashboard scenario launcher for design review (`ui-scenario.ts`, `just ui-scenario <scenario>`).
- `dsl/`: the code-rule DSL benchmark.
- `feedback/`: the weekly feedback digest (`digest.mjs`): a week's GitHub issues, rule misfires by kind, rule and pack,
  and the open backlog, as a page of the product team's Notion database. `.github/workflows/feedback-digest.yml` runs
  it every Monday; `--print` prints the digest instead.
- `generate/`: generators of committed engine and host data and the DSL reference page (manifest: `generate/README.md`).
- `host/`: real-host runs: OMP install, host smoke, lifecycle and auth checks, the Windows install smoke.
- `pack-data/`: the derivation of the packs' derived test data from their pins (`derive.ts`, `npm run fixtures:derive`),
  with one recipe per pack in `recipes/`, the runtimes it installs (`runtimes/`, `runtimes.ts`), the ESLint
  expected-findings helper the ESLint-based generators share (`eslint-expected.mjs`) and the renamer that writes the
  ECMA rename variants (`rename/`), and the writer of each tool's `provenance.json` and `LICENSES.md` the generators
  call last (`provenance.mjs`); a maintainer's sandbox and the bump workflow run it, never pull request CI.
- `release/`: the release. The Release pull request (`.github/workflows/release-pr.yml`): every package's version,
  core's when what it ships changed, and core's tag on merge. The rule packs' release
  (`.github/workflows/release-packs.yml`): which packs to publish, the publish gate and packing, publishing under
  `latest`, putting a pack back, and the lockfile sync after `changeset version`. The dist-tag check after a publish.
- `review/`: the guidance review (`just rerail-guidance-review`): every rule pack rule run on its examples and upstream
  fixtures, with the steer the coding agent reads, in a local page where each rule's guidance is approved or sent back
  with comments.
- `samples/`: the packs' real-code sample sets (`format.ts` says what one holds), a library API with no command: the
  sample model and archive (`samples.ts`), staging a set into `publish/samples/` (`pack.ts`), the checks the publish
  makes of staged samples (`publish.ts`), the clean-sample relevance selection (`relevance.ts`), the ESLint test set's
  conversion (`convert-eslint-test-set.ts`), and the recorded source licences with the `LICENSES.md` and `SAMPLES.md`
  they write (`licences.ts`; docs/upstream.md, "Samples").
- `s3/`: the packs' pack data archives in S3 (`pack-fixtures.mjs`: `npm run fixtures:pull` and `fixtures:stage`, the
  `pack`, `upload` and `confirm` steps of `.github/workflows/pack-data-publish.yml`, and `fixtures:push`, a
  maintainer's upload outside that workflow; `pack-data-publish.mjs`: that workflow's locks and sticky comment, the
  request workflow's waiting state with its `replace` recommendation included; the canonical
  tar and lock format in `test-data-archive.mjs`; docs/upstream.md, "Publishing pack data"), with the messages its
  consumers give in Python (`pack_fixtures.py`).
- `upstream/`: the upstream release check (`releases.ts`, `npm run upstream:releases`) and the pin bump (`bump.ts`,
  `npm run upstream:bump`) the upstream workflows run (docs/upstream.md).
