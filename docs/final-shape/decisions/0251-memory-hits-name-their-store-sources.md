# 0251: what a run takes from process memory, it names in its manifest

Status: built (2026-11-15). Amends decisions 0073 (the prepared program kept), 0075 (groups of automata kept) and
0102 (the manifest names what the store's writer stores).

## Intro

A run's manifest names the store sources the run depends on: collection keeps them while it keeps the run, and two
runs over the same inputs record the same manifest. A run that reads the prepared program or a group of automata
from the store names their sources as it reads them; one that stores them names what it stored.

A process that runs more than once (a daemon, `repeat`) takes both from memory in its second run: the prepared
program from the process's cache, the groups from the searchers compiled once a process. Neither named anything.
The second run's manifest left out the programs' texts and images and the `dfa1:` group sources, and was a new
manifest: the next process, reading them from the store, named them again and recorded another.

## Decision

A hit in process memory names what the store read would have:

- **The prepared program** records, by store path, the sources its texts are kept under (`Prepared::documents`):
  when its write lands, or when it is read from the store. A run that takes it from memory names this store's. Where
  they are unknown (its write still queued, or kept for another store), the queued write is settled, then the
  pointer alone is read (`prepared::documents`, `program_text::sources`): no program is read.
- **A group of automata** taken from the process's kept groups, or from searchers compiled before, is named in this
  run's cache (`DfaCache::held`, an indexed source lookup) as a read of it would be. A group the store does not keep
  (determinized for another store) is put there.

## What this gives up

- A memory hit costs a source lookup per group, and for a program first held for another store one pointer read.

## Tests

- `a_second_run_in_one_process_records_the_first_runs_manifest` (engine): a process of one run, then a process of
  two; the second process records no manifest the first did not. Two new before (texts, images and the group), one
  with only the prepared program fixed.
