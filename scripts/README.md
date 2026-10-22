# scripts

Repository scripts, grouped by what they are for. Their tests live in `test/scripts/`.

- `build/`: steps of `npm run build` and `npm pack` (copy the UI, require cargo, repair SQLite, stamp plugin
  versions).
- `check/`: the checks `npm run check` and `check:*` run, including the layer boundary lint
  (`check-boundaries.mjs`), the Windows checkout check (`check-windows-paths.mjs`: every tracked path short
  enough and named so Windows can check it out), the internal reference check (`check-internal-refs.mjs`: no
  internal ticket ID in a tracked path or text) and the licence allowlist.
- `dev/`: a development install from this checkout (`just rerail-dev`), its restore, nuke and smoke runs.
- `dsl/`: the code-rule DSL benchmark.
- `generate/`: generators of committed engine and host data and the DSL reference page (manifest: `generate/README.md`).
- `host/`: real-host runs: OMP install, host smoke, lifecycle and auth checks, the Windows install smoke.
- `release/`: the release. The Release pull request (`.github/workflows/release-pr.yml`): every package's version,
  core's when what it ships changed, and core's tag on merge. The rule packs' release
  (`.github/workflows/release-packs.yml`): which packs to publish, the publish gate and packing, publishing under
  `latest`, putting a pack back, and the lockfile sync after `changeset version`. The dist-tag check after a publish.
- `review/`: the guidance review (`just rerail-guidance-review`): every rule pack rule run on its examples and upstream
  fixtures, with the steer the coding agent reads, in a local page where each rule's guidance is approved or sent back
  with comments.
