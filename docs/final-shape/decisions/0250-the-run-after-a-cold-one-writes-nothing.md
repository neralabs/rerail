# 0250: the run after a cold one writes nothing

Status: built (2026-11-15). Amends decision 0246 (rerail's own files), the whole-run cutoff (t23 §4), the planning
choices (decision 0090 §5) and the observer's collection (`observe::collect`).

## Intro

A warm run is one over inputs an earlier run read, all unchanged. Under `RERAIL_WARM` it must write nothing: no
store row, no manifest, no observer answer. From the third run on this held. The second run, the first warm one
after a cold run on a fresh store, still wrote: new manifests, planning choices, and the observer's answers and
canonical forms. The third run was clean only because the second had written what the first should have.

The user asked that the second run be warm too, test first (2026-11-15).

## Decision

Each write the second run made had a cause in the first run, and each is fixed there:

| The second run wrote | Because the cold run | Now |
|---|---|---|
| New manifests, same plan and sources, another digest | named a source no later run reads: a chunk's tree counts, written though no reader has used them since frames carry their own statistics | The counts' writer is removed with its reader (`own_counts`, `record_own_counts`, `Session.own`) |
| The same: its outer manifests named fewer plans | ran its nested runs; the warm run reused them whole (`reuse::reused`) and named none of their plans | A reused run names the plans its record holds (the record's `plan` lines), as the run that made it did. Kept constants keep their run's plans too (`constants:` records, key v2) |
| Planning choices | planned the cross part's programs. The warm run keeps every stratum, so it plans a program of no rules, a key no run planned before | A program of no rules (and no derived-again groups) has no choice to make: none is looked up or kept |
| Observer answers and canonical forms under the corpus root | recorded them, and its collection then deleted them: collection ran before the run settled the corpus root's cursor, so no stored root held them | Collection keeps an unheld answer or canonical form while it is young (an hour, `UNHELD_GRACE_NS`). Canonical records now hold their recording time |
| The same, under the rule library | watched inner roots first (the library, then the worktree around it). Each root settled keeping only what was checked as its own | A wider root first asked takes in the roots inside it this process asked: their answers, and the earliest of their cursors |
| Cursor moves for `$TMPDIR` | canonicalized and listed phase 0's scratch corpus, which watched the whole temporary directory, other programs' files included | Phase 0's scratch (`$TMPDIR/rerail-phase-0-…`) is rerail's own (0246). An own directory's listing watches nothing above it |

### The root that takes others in

A root first asked in a run, with no cursor stored, is checked by `stat` and settled at the run's end. Settling
forgets what was not checked under it, then stores its cursor. With nested roots this lost answers two ways:

- The inner root, settled apart, forgot what was checked under it as the outer root's.
- The outer root forgot what was checked before it was asked, in this run or an earlier run of the process.
  Phase 0's runs come before the corpus run.

Now, when a root is first asked, every root inside it that the process asked before, on the same volume and with a
cursor, is taken in:

- Its answers count as checked under the new root. This run's untrusted inner roots are removed and settled as the
  new root.
- The new root's cursor is the first such root's cursor, the earliest. A cursor taken earlier reports more
  changes, so every answer taken in holds as of it.

One cursor is stored, the outer root's.

### Collection beside a run

Collection works on its own thread, beside the run's end, and maybe in another process. It cannot wait for another
run's roots to settle. So it keeps what no stored root holds while it is young: a run lasts minutes, the grace is an
hour. Older unheld answers go as before.

## What this gives up

- **Unheld answers stay up to an hour longer.** They cost only space.
- **An outer root's cursor is older than its own first ask.** Its first warm run replays the journal from there:
  more events to read, never fewer.

## Tests

- `the_run_after_a_cold_one_writes_nothing` (engine): a cold run, then one under `RERAIL_WARM=list`, each a
  process of its own. The warm one exits cleanly and records no manifest of its own. It failed with 18 faults before
  the planning-choices fix.
- `a_first_run_keeps_what_it_checked_under_nested_roots` (store): an inner root watched before the root around it,
  answers asked before and after. All are kept, and one cursor is stored.
- `a_first_process_keeps_what_an_earlier_run_checked_under_an_inner_root` (store): the same across two runs of one
  process.
- `rerails_own_files_watch_nothing` (store): listing, stat, digest and canonical of own files ask no root.
- `collection_drops_what_no_run_asks_any_more` (store): a young unheld answer and canonical form are kept.

Measured on dl10 (`layer-violations.rr`, fresh store): the second run went from exit 3 (5 to 26 faults) to exit 0
with no faults, 0.57 s wall. The third run is clean too, and the findings are unchanged.
