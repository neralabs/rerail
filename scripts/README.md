# scripts

Repository scripts, grouped by what they are for. Their tests live in `test/scripts/`.

- `build/`: steps of `npm run build` and `npm pack` (copy the UI, require cargo, repair SQLite, stamp plugin
  versions).
- `check/`: the checks `npm run check` and `check:*` run, including the layer boundary lint
  (`check-boundaries.mjs`), the Windows checkout check (`check-windows-paths.mjs`: every tracked path short
  enough and named so Windows can check it out) and the licence allowlist.
- `dev/`: a development install from this checkout (`just rerail-dev`), its restore, nuke and smoke runs.
- `dsl/`: the code-rule DSL benchmark, data-layer rule inventory (`data_layer_rule_audit.py`), test survey
  (`data_layer_test_survey.py`), pytest recorder (`data_layer_record.py`, `--batch 1`, `--batch 2`, or `--batch 3`),
  batch-two analyzer adapters (`data_layer_batch_two.py`), supplemental probe recorder
  (`data_layer_probe_record.py`), and shared-suite recorder
  (`data_layer_suite_record.py`, `--batches 1,2,3` for thirty checks).
  Third-batch recording uses `data_layer_batch_three.py`; its probes use `data_layer_probe_three.py`.
  `data_layer_generate_batch_three.py` regenerates ten policy documents after verifying pinned source and artifacts.
- `generate/`: generators of committed engine and host data and the DSL reference page (manifest: `generate/README.md`).
- `host/`: real-host runs: OMP install, host smoke, lifecycle and auth checks, the Windows install smoke.
- `release/`: the rule pack release (`.github/workflows/release-packs.yml`): which packs to publish, the publish gate
  and packing, publishing under `next`, promotion to `latest`, and the lockfile sync after `changeset version`.
