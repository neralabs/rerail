# Recursion in `calls.rr`'s real edits (decision 0328)

What a maintained recursive stratum must handle, measured on the O125 replay
(`~/orca/rerail-shared/o125-replay`, 77 files, steps 01–08) from the engine's own derivations:

1. `prov.py tmp/o125/calls-policy.rr tmp/rec/calls-prov.rr tmp/rec/meta.json`, run from this directory, adds
   one rule `drv_i` per rule of a recursive component. Its columns are the rule's whole binding: wildcards and
   constants become variables. It also adds one copy `cp_r` per relation of another stratum those rules read. All of
   these are forced into the cross part by `prov_all`, a count over every document.
2. `POLICY=tmp/rec/calls-prov.rr zsh scripts/profiling/o125/recursion/dump.sh NN` for NN in 00–08: a cold
   strict run of each step, with every relation dumped (`RERAIL_REL_DIR`).
3. `python3 scripts/profiling/o125/recursion/rec.py` (about 1 min, 2 GB) prints, per state, the rows on a cycle
   of the derivation graph. Per edit, it prints what recheck alone keeps wrongly, DRed's overdelete, and
   yes/no/maybe's candidates, with yes/no/maybe checked equal to the next state.

`rec.py` takes each relation as the heads of its derivations, which is the program's meaning. The engine's dump
agrees, except for `found`, which it derives by demand (`.pragma "magic-transform"`).

With the per-file memo, a file-local relation's dump holds only the edited files' rows, and a stratum whose
inputs did not change writes no dump: `rec.py` takes it from the last step that did. Every step runs on one store:
a store per step interns symbols and nodes apart, and rows would not compare.

`rec.py` also prints, per edit, the candidates on an old cycle (where one probe can keep a row wrongly) and a sound
variant, `prove`: a candidate stays only when its cone is derived again from below. The one-probe result depends on
the probe order; run it under a few `PYTHONHASHSEED`s.

## Other programs (the light pass, 2026-11-17)

- **`workload.rr`:** `REC=tmp/recw POLICY=tmp/recw/workload-prov.rr dump.sh NN`, then `REC_DIR=tmp/recw rec.py`.
  `prov.py` leaves out components with a lattice relation (`cv`, which is file-local).
- **a data-layer policy over dl10:** `expand.py docs/rule-syntax/data-layer/P.rr tmp/recd/P.rr` reads its
  `#include`s in; `prov.py` as above; then `LOGIC=1 STATES=tmp/recd/states REC=tmp/recd/P POLICY=tmp/recd/P/prov.rr
  dump.sh NN` for each tree `tmp/recd/states/NN` (`logic` over the whole tree, the policy's `.scope` dropped).
- **a dense closure:** `MAINTAIN_EDGES=DIR target/profiling/examples/maintain 300 8` dumps the spike's `edge` per
  edit; `ynm_reach.py DIR` counts one probe, the proof and DRed on its `reach`. A ring that loses its entry, where
  one probe is wrong, is a two-file `DIR` by hand (decision 0328, "A correction").
