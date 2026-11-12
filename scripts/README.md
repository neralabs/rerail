# scripts

Repository scripts, grouped by what they are for. Their tests live in `test/scripts/`.

- `build/`: steps of `npm run build` and `npm pack` (copy the UI, require cargo, repair SQLite, stamp plugin
  versions), and the documentation site GitHub Pages serves (`docs-site.mjs`, `npm run build:docs-site`: `docs/` as
  pages in `dist-docs-site/`, with each page's Markdown twin, `llms.txt` and the code rule's JSON Schema for agents;
  `.github/workflows/docs-site.yml` publishes it). The hook and event-stream bundles share `npm run build:hook-bundle`,
  whose banner defines `require` because the bundled `yaml` package (Merge Assist's `.rerail/policy.yaml`) is
  CommonJS and requires Node built-ins, which esbuild's ESM output cannot.
- `check/`: the checks `npm run check` and `check:*` run, including the layer boundary lint
  (`check-boundaries.mjs`), the Windows checkout check (`check-windows-paths.mjs`: every tracked path short
  enough and named so Windows can check it out), the internal reference check (`check-internal-refs.mjs`: no
  internal ticket ID, and no `nera-` identifier outside its list of public ones, in a tracked path or text), the CI
  path filter check (`check-ci-paths.mjs`: every file the jobs `ci.yml`'s path filters skip import, from the entry
  points it lists, matches their filter), the licence allowlist, and the pre-commit check (`pre-commit.sh`: CI's lint
  and type-check job, before each commit).
- `deps/`: pins that follow a dependency (`sync-pins.mjs`, `npm run deps:sync`): the OMP host and its release digests
  follow the OMP SDK, the ast-grep differential test's reference follows the ast-grep CLI, and `reviewer.yml`'s
  `reviewer_ref` follows the commit its `uses:` pins. `.github/workflows/dependabot-pins.yml` runs it on Dependabot's
  npm pull requests; `npm run check` fails on drift.
- `dev/`: a development install from this checkout (`just rerail-dev`), its restore, nuke and smoke runs, the
  dashboard scenario launcher for design review (`ui-scenario.ts`, `just ui-scenario <scenario>`), and the pre-commit
  hook's installer (`install-git-hooks.mjs`, run by `npm install`'s `prepare`).
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
  (`.github/workflows/release-packs.yml`): which packs to release, the publish gate and packing, tagging each
  release before it is published, publishing under `latest`, putting a pack back, and the lockfile sync after
  `changeset version`. The dist-tag check after a publish,
  and the GitHub Release of each released version from its changelog section. The release notes: core's section
  written from the entries pending in `.changelog/` and each pack's from its `pack.json` and pending changesets, both by
  change type, each pack rule by rule, and the check that every pending entry and changeset names its type, that a
  changeset under Added, Removed or Fixed opens with its rule, that an entry or changeset a
  pull request adds is written for the person upgrading, that no entry is written into `CHANGELOG.md` by hand, and that a release's section leads with a paragraph.
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
  maintainer's upload outside that workflow; `pack-data-publish.mjs`: that workflow's locks and comment; the canonical
  tar and lock format in `test-data-archive.mjs`; docs/upstream.md, "Publishing pack data"), with the messages its
  consumers give in Python (`pack_fixtures.py`).
- `upstream/`: the upstream release check (`releases.ts`, `npm run upstream:releases`) and the pin bump (`bump.ts`,
  `npm run upstream:bump`) the upstream workflows run (docs/upstream.md).
