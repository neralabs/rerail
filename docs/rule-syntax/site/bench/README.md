# Benchmark and run scripts for main's pack

These scripts measure the Datalog engine against main on main's own Python rules. They were scratch scripts of one
session; they live here so they are not lost.

| File | What |
|---|---|
| `bench-shapes.sh <full\|one-file\|one-rule> <engine>` | Main 0.1.3 against the engine, step by step: cold, warm, and the incremental edits. |
| `bench-steps.sh` | The older full-shape table that `bench-shapes.sh full` replaces. |
| `rules-bisect.py <rules> <corpus> <always.rr> <pattern>` | The smallest set of other rules that, with one rule, still gives a finding (ddmin). |
| `corpus0.sha256` | The 300 Python files of the benchmark corpus, with their hashes. |

The inputs are outside the repo, in this Mac's shared data (`SP`, default
`${RERAIL_SHARED:-~/orca/rerail-shared}/bench/step-vs-main`; never /tmp or a session scratchpad, which macOS wipes:
the inputs were lost there on 2026-11-14). The scripts only read it; each run's outputs go in `$OUT/run-<shape>`
(default `./tmp/bench`, beside the caller), which a run empties first. It holds:

- `corpus0`, `corpus1`, `corpus10`, `corpus50`: the 300 files, and the 1, 10 and 50 file subsets. The files are from
  the data-layer repository; `corpus0.sha256` names each one, so a copy can be checked.
- `pack-B` and `home-B`: main's Python pack 0.1.8 with B005 off (317 rules on), installed for main's CLI.
  `../../packs/python-main/source` is a copy of that pack's `pack.json` and `rules/`.
- `rules/B-now`, `rules/B-one`: the `.rr` rules the engine runs in the full and one-rule shapes, copied from the
  hand-written `../../packs/python` (B-now: the 317 ids that are on in pack-B; B-one: B101), with `rs-now` holding
  copies of `../../lib` and `../../packs`. Not `packs/python-main`: that is the translator's output from the JSON,
  about five times the text and six times the components on one file, and a different comparison.

`CLI` names main's CLI (`@neralabs/rerail` 0.1.3's `dist/cli/cli.js`), by default the copy in `$SP/main-pkg`.

`../run_main_pack.sh` runs each rule of `packs/python-main` alone over a corpus under the caps, and
`../capped.sh` is the cap (2 minutes and 2 GB by default) it and these scripts run under.
