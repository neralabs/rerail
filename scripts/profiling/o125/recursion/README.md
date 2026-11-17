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
