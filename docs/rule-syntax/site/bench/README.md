# Benchmark and run scripts for main's pack

These scripts measure the Datalog engine against main on main's own Python rules. They were scratch scripts of one
session; they live here so they are not lost.

| File | What |
|---|---|
| `bench-shapes.sh <full\|one-file\|one-rule> <engine>` | Main 0.1.3 against the engine, step by step: cold, warm, and the incremental edits. |
| `bench-steps.sh` | The older full-shape table that `bench-shapes.sh full` replaces. |
| `rules-bisect.py <rules> <corpus> <always.rr> <pattern>` | The smallest set of other rules that, with one rule, still gives a finding (ddmin). |
| `make_overrides.py` | Wrote the first `overrides/` files from the generated rules. Kept for reference. |
| `corpus0.sha256` | The 300 Python files of the benchmark corpus, with their hashes. |

The inputs are outside the repo. `SP` (default: the session's scratchpad) holds:

- `corpus0`, `corpus1`, `corpus10`, `corpus50`: the 300 files, and the 1, 10 and 50 file subsets. The files are from
  the nera-data-layer repository; `corpus0.sha256` names each one, so a copy can be checked.
- `pack-B` and `home-B`: main's Python pack 0.1.8 with B005 off (317 rules on), installed for main's CLI.
  `../../packs/python-main/source` is a copy of that pack's `pack.json` and `rules/`.
- `rules/B-now`, `rules/B-one`: the `.rr` rules the engine runs in the full and one-rule shapes.

`CLI` names main's CLI (`@neralabs/rerail` 0.1.3's `dist/cli/cli.js`).

`../run_main_pack.sh` runs each rule of `packs/python-main` alone over a corpus under the caps, and
`../capped.sh` is the cap (2 minutes and 2 GB by default) it and these scripts run under.
