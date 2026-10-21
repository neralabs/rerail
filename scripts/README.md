# scripts

Repository scripts, grouped by what they are for. Their tests live in `test/scripts/`.

- `build/`: steps of `npm run build` and `npm pack` (copy the UI, require cargo, repair SQLite, stamp plugin
  versions).
- `check/`: the checks `npm run check` and `check:*` run, including the layer boundary lint
  (`check-boundaries.mjs`), the Windows checkout check (`check-windows-paths.mjs`: every tracked path short
  enough and named so Windows can check it out) and the licence allowlist.
- `dev/`: a development install from this checkout (`just rerail-dev`), its restore, nuke and smoke runs.
- `dsl/`: the code-rule DSL benchmark.
- `generate/`: generators of committed engine and host data and the DSL reference page (manifest: `generate/README.md`).
- `host/`: real-host runs: OMP install, host smoke, lifecycle and auth checks, the Windows install smoke.
- `release/`: the rule pack release (`.github/workflows/release-packs.yml`): which packs to publish, the publish gate
  and packing, publishing under `next`, promotion to `latest`, and the lockfile sync after `changeset version`.
- `review/`: the guidance review (`just rerail-guidance-review`): every rule pack rule run on its examples and upstream
  fixtures, with the steer the coding agent reads, in a local page where each rule's guidance is approved or sent back
  with comments.
- `s3/`: pack test data in S3 (`pack-fixtures.mjs`, `npm run fixtures:pull` and `fixtures:push`; CONTRIBUTING.md,
  "Pack test data"), with the messages its consumers give in Python (`pack_fixtures.py`).
