# scripts

Repository scripts, grouped by what they are for. Their tests live in `test/scripts/`.

- `build/`: steps of `npm run build` and `npm pack` (copy the UI, require cargo, repair SQLite, stamp plugin
  versions), and the documentation site GitHub Pages serves (`docs-site.mjs`, `npm run build:docs-site`: `docs/` as
  pages in `dist-docs-site/`, with each page's Markdown twin, `llms.txt` and the code rule's JSON Schema for agents;
  `.github/workflows/docs-site.yml` publishes it).
- `check/`: the checks `npm run check` and `check:*` run, including the layer boundary lint
  (`check-boundaries.mjs`), the Windows checkout check (`check-windows-paths.mjs`: every tracked path short
  enough and named so Windows can check it out), the owned built-ins check (`check-owned-builtins.mjs`: one table of
  the Node built-ins only one module in `src/` may use, each in the module that makes it safe on Windows: child
  processes through `src/engine/child-process.ts`, which hides their windows and starts a program only by its
  absolute path; renames through `src/engine/file-replace.ts`, which retries a rename while another process holds the
  file; in the modules that key records by a checkout path, realpath through
  `src/engine/catalog/checkout-control.ts`, which spells a Windows path as Git does; and `O_NOFOLLOW` through
  `src/engine/no-follow.ts`, which refuses a symlink on Windows too), the folder removal check
  (`check-folder-removal.mjs`: tests remove the folders they make only through `test/support/platform/remove-folder.ts`,
  which names a process the test left running in one and waits out any other holder on Windows; it and the owned
  built-ins check read files through `source-files.mjs`, which parses them and lists the files Git would commit), the internal
  reference check (`check-internal-refs.mjs`: no internal ticket ID, and no `nera-` identifier outside its list of
  public ones, in a tracked path or text), the CI path filter check (`check-ci-paths.mjs`: every file the jobs `ci.yml`'s path filters skip import, from the entry
  points it lists, matches their filter), the licence allowlist, and the pre-commit check (`pre-commit.sh`: CI's lint
  and type-check job, before each commit, on the working tree merged into the local `origin/main` when the branch is
  behind it, as CI checks the pull request merged into main; `merged-checkout.mjs` makes that throwaway checkout).
- `deps/`: pins that follow a dependency (`sync-pins.mjs`, `npm run deps:sync`): the OMP host and its release digests
  follow the OMP SDK, the ast-grep differential test's reference follows the ast-grep CLI, and `reviewer.yml`'s
  `reviewer_ref` follows the commit its `uses:` pins. `.github/workflows/dependabot-pins.yml` runs it on Dependabot's
  npm pull requests; `npm run check` fails on drift.
- `dev/`: a development install from this checkout (`just rerail-dev`), its restore, nuke and smoke runs, the
  dashboard scenario launcher for design review (`ui-scenario.ts`, `just ui-scenario <scenario>`), and the pre-commit
  hook's installer (`install-git-hooks.mjs`, run by `npm install`'s `prepare`).
- `dsl/`: the code-rule DSL benchmark, data-layer rule inventory (`data_layer_rule_audit.py`), test survey
  (`data_layer_test_survey.py`), pytest recorder (`data_layer_record.py`, `--batch 1`, `--batch 2`, `--batch 3`, or `--batch 4`),
  batch-two analyzer adapters (`data_layer_batch_two.py`), supplemental probe recorder
  (`data_layer_probe_record.py`), and shared-suite recorder
  (`data_layer_suite_record.py`, `--batches 1,2,3,4` for forty candidates).
  Third-batch recording uses `data_layer_batch_three.py`; its probes use `data_layer_probe_three.py`.
  `data_layer_generate_batch_three.py` regenerates ten policy documents after verifying pinned source and artifacts.
  Fourth-batch recording uses `data_layer_batch_four.py`; its probes use `data_layer_probe_four.py`.
  `data_layer_generate_batch_four.py` regenerates its candidates; the fourth-batch report records unresolved gaps.
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
- `ship/`: the shippability bar (`shipbar.ts`, `npm run bench:ship -- prepare|run|verdict …`): whether a candidate
  engine is no worse than main's on every rule pack main ships, as two verdicts, the candidate's packs as they ship
  through the product's path (`product-run.ts`) and the hand-written `.rr` rewrites through `rerail-engine
  run-policies`. Per pack, on its fixtures and on the real-code corpus `corpora.json` pins, each side runs N times
  interleaved on pinned cores under the same memory and time caps, and the verdict (`verdict.ts`) fails a metric (wall,
  CPU and peak RSS, cold and warm) above 1.10 times main's median, a lost true positive or a new false positive against
  the upstream recording, a run main completes that the candidate does not, and a rule main ships that the candidate
  cannot run. A cell failing at the median runs `--confirm` more repetitions (a metric passing them is noisy: reported,
  not counted), and a confirmed slowdown is attributed to rules by timing each rule alone (`--drill`). `prepare` exports
  the baseline and the corpora anywhere; `run` measures on Linux (taskset, GNU time, a cgroup), macOS (`time -l`, no
  CPU affinity, a polled memory cap: `platform-watch.mjs`) or Windows (a job object: `platform-win.cs`), as
  `platform.ts` sets out, each run row naming how it was pinned, capped and measured, signed in to AWS for the pack
  data; `run` and `verdict` write `report.json` (`report-model.ts`, schema `ship-no-ship/report/v1`) and `report.md`
  (`report.ts`): one row per pack naming its worst metric and the rule behind it, each pack's metrics, rules and raw
  runs behind `<details>`. Beside the gross figures, and never judged (`metric.ts`, `metric-report.ts`): each side's
  start-up (its empty rule set on the same corpus, `fixed.jsonl`), net time and net ratio, the product path's load /
  compile / evaluate phases (compile per rule, evaluate per rule per syntax node), the corpus's files, bytes, lines,
  tokens and engine node counts (`<out>/workload/`), and in the drill-down each rule's multiple of the pack's median
  rule.
  `waits` (`waits.ts`, Linux) measures the waits a user experiences (W1–W8) on both checkouts through their built
  Claude Code hook, CLI and daemon, each side in its own fresh home: a session replayed from public git history
  (`waits-session.ts`, pinned in `waits.json`; `waits --prepare` builds it where the history is), first sessions,
  scans, daemon and CLI start-up, an upgrade's first session, status while an audit runs, and the step benchmarks'
  edit sequence in three shapes (`waits-steps.ts`). `waits-model.ts` judges them (distributions by the bootstrap CI of
  the p50 and p95 ratio, single waits by the median, absolute limits from `waits.json`) into the report's `waits`
  section, ids `wait/<W>/<scenario>`.
- `upstream/`: the upstream release check (`releases.ts`, `npm run upstream:releases`) and the pin bump (`bump.ts`,
  `npm run upstream:bump`) the upstream workflows run (docs/upstream.md).
