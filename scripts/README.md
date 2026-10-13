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
