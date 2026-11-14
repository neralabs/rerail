# 0197: Warm mode: a truly warm run writes nothing, and any write fails it

Status: built (2026-11-14); its first findings below, two fixed and the rest open. Follows decision 0183 (a warm
batch recomputes nothing) and uses 0104's statement trace (`RERAIL_SQL_STRICT`).

## Intro

A run is truly warm when its store already holds everything it computes, and its rules and inputs have not changed
since. Such a run should only read. Measuring warm runs (0183) shows how much they cost, but not why: a warm run
that quietly computes and writes something again still gives the same findings, so nothing fails.

`RERAIL_WARM` is a development flag that turns each such recomputation into a failure:

```
# Warm the store, then check that a second run only reads.
rerail-engine run-policies dl1 out-1 policies...              # cold, or warm already
RERAIL_WARM=1 rerail-engine run-policies dl1 out-2 policies...   # aborts at the first write, with its stack
RERAIL_WARM=list rerail-engine run-policies dl1 out-2 policies...  # reports them all, exits with status 3
```

It is only for runs known to be warm. Over a changed tree it fails, as it should.

## What fails a warm run

| Fault | Where it is caught |
|---|---|
| A SQLite write committed, on any connection: the store, its `.files` observer store, the cost samples, an in-memory database | A commit hook on every connection (`profile::warm_commit`). SQLite calls it only for a transaction that wrote or took the write lock. The store's atoms set their own hook, which calls it too. |
| A write queued for the store's writer | `ContentStore::write_back` and the like (`write_back::queue`), so the stack is the caller's, not the writer thread's. |
| The whole-run cutoff asked for and not applying | `reuse::miss`: a rule file, a document or a read changed, or no record. A run that did not ask (a session's) is checked by its deltas instead. |
| A changed document, or a non-empty delta | `input_delta::changed` and `delta`; a maintained part's derived `__minus`, `__plus`, `__del`, `__ins`, `__gone` relations with rows. |

The mode is `profile::warm_mode()`, beside `strict()`. `RERAIL_WARM` is one of `reuse::NOT_OUTPUTS`: it changes how
a run is checked, not what it outputs. If it were in the run's key, no checked run would ever find the record of
the run that warmed the store.

`list` prints each kind's first fault with its stack and each distinct fault once. When the process exits it
prints the counts and exits with status 3.

## First findings: the `dl1` batch

These are from 44 data-layer policies over one Python file (`dl1`, decision 0183), `run-policies`, CPU, strict
SQL, debug build. A rebuild changes the engine's id, which is in the run's key, so each check came after one cold
run and one warming run.

The whole-run cutoff applied in every warm run: no policy was planned or evaluated. Findings were the same in
every run. Every write came from setup around the run, not from evaluation:

| Found | Writes a warm batch | Status |
|---|---|---|
| The store's schema check applied all 124 `CREATE` statements of `SCHEMA` to a scratch in-memory database, once a process, to learn the object names it expects | 124 | **Fixed**: the names are read from `SCHEMA`'s text (`schema_objects`). The test `schema_objects_are_what_schema_creates` applies the schema and compares the two. |
| The observer bumped a stored `epoch` counter on every open, to rotate which 1/64 of trusted files it hashes again | 1 | **Fixed**: the epoch is the second the process opened in. Consecutive processes still take consecutive slices. Lost: the counter's guarantee that 64 processes in a row cover every file once. Two processes in the same second check the same slice. |
| The observer moves and stores each watched root's journal cursor on every run, though the journal names nothing changed under it | 13 | Open (below) |
| One canonical path form (`docs/rule-syntax/lib/python`) is written again on every run | 1 | Open |

With the cutoff off (`run-rr` with `{"reuse": false}`), a warm run plans and evaluates, and maintains from what the
last run kept. Its deltas were all empty. It still writes:

- the run's lifecycle in the store: its manifest, its hold on its plans and the release, a source's pins, the
  combined program's pins and its text, the run's atoms;
- empty jobs: `frames, 0 bytes`, `the memo's entries, 0 bytes`, `the pending atoms`, `a wait for the writer`;
- `the tree counts` (three, 44 bytes each), `the files' stats`;
- the cost model's samples (`~/.cache/rerail/costs.sqlite`: one insert and a trim);
- the observer's journal cursors, as above.

## Open

- **The journal cursor.** Storing the cursor only when the journal names a change under the root would make a warm
  run write nothing. The next run would then replay the journal from an older cursor. That is correct, since the
  replay is a superset, but it costs more, and on a volume where builds write all day the replay grows until
  something else writes. The other option is to move the cursor only with a write the run makes anyway. Neither
  is measured. The roots are also wider than the run's reads: the work tree itself is watched, so its output
  directories and the store's own files show up as changes.
- **The canonical form written again.** The kept form is not found and is resolved again each run, though it is
  unchanged.
- **Empty jobs.** A queued job with nothing to write still opens a write transaction. Not queueing it is the fix.
- **The run's lifecycle writes** (manifest, holds, pins). These are collection's generations, made per run. If a
  warm run should leave the store byte for byte as it was, a run that changed nothing would keep its last
  manifest and hold nothing new. That is collection's design (0053) to settle.
- **The cost samples** are training data for the planner's cost model. A warm run that plans adds a sample. In
  warm mode that counts as a write, as it should, but whether a warm run should sample at all is open.
