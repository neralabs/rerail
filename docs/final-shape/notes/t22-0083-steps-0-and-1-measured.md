# t22. Decision 0083's first two checks, measured on today's engine

## Intro

Decision 0083 (on `final-rerail-shape-phased-facts`) says a program's answers and cost must not depend on how
code is split into files. It also says that adding one import probably misses every file's per-file memo. Its
order of work starts with two checks on today's engine, before anything is built:

- **Step 1.** Add one import to one file, and count the files the per-file memo misses.
- **Step 0.** Move definitions between modules (imports rewritten), and compare the answers and the work.

This note gives both results for the session building 0083. Nothing in 0083 is edited here.

Setup, for both:
- the 84-file data-layer slice (267 documents with typeshed stubs);
- `calls_cx` (`RERAIL_PYTHON_PROGRAM=calls_cx`), with the per-file memo and the cross-file memo per import
  component (decision 0076);
- engine at the merge of `final-rerail-shape` into `investigate-bend-vs-datalog` (996ef7e1c);
- CPU is user + sys from `/usr/bin/time`, with the main thread's figure from the run's report;
- the machine was loaded (load average 60–95), so wall times are long and only CPU is compared.

## Step 1: one added import misses one file

The edit is `import os` added after the imports of `l1100_exceptions/exceptions.py`, on a store primed with
the unedited corpus.

| | per-file memo | cross-file components | CPU main thread | cross part | RSS |
|---|---|---|---|---|---|
| prime (cold) | recorded | 0 found, 133 derived | 128.8 s | 81.7 s | 842 MB |
| warm (no edit) | 267 of 267 | 133 found | 11.3 s | 2.5 s | 371 MB |
| one added import | **266 of 267** | 92 found, **41 derived** | 94.2 s | 79.7 s | 846 MB |

- The inference in 0083 §5 does not hold today. The per-file memo's digest hashes the file-local program
  (`memo::program_digest(&split.local)`: its rules, declarations, facts, inputs and outputs). `dir_entry`,
  `loaded` and bridged rows do not reach that program's facts. So the import misses only the edited file.
- The cost is all in the cross-file part. Each import component's key covers the components it imports. The
  edited file's component changes, so every component that reaches it is derived again: 41 of 133 components, and
  80 s of the 94 s.
- So the edit's cross part costs what a cold one does (79.7 s against 81.7 s), although its delta is one import. That is 0083's case for symbol keys
  and maintenance by deltas (§2, §3), not a plan-digest problem.

## Step 0: moving definitions keeps the answers, not the work

`benchmark/layout-invariance/move.py` moves the 8 definitions the Jedi oracle's calls target most, one per file:
- Each definition goes to a new module next to its file, `_moved_<k>.py`.
- The new module copies the file's imports, and imports from the file the names it reads.
- The old file keeps `from <pkg>._moved_<k> import <name>` where the definition was, so importers are unchanged.
- A map takes each new `(path, line)` back to the old one.

`benchmark/layout-invariance/compare.py` maps the moved run's answers back and compares them. An answer is a
call site, its kind (`def`, `class` or `open`) and its target. They come from the benchmark test with
`RERAIL_PYTHON_CALLS_ALL=<file>`.

Moved: `_legacy_delegate_stub`, `BugTestItem`, `_lazy_runtime` (reads two names of its old module),
`MockExecutionContext`, `error_sanitization_enabled` (reads `_thread_local`), `Product`,
`_TxnPermissionGuardrailRow` and `_FakePlan`.

**Answers.** There are 10,773 answers in each layout. After mapping, none differ. That includes every call into a
moved definition, which now resolves through a re-export.

**Work, as an incremental run** on the store primed with the original layout:

| | per-file memo | cross-file components | CPU main thread | cross part |
|---|---|---|---|---|
| original layout, warm | 267 of 267 | 133 found | 11.3 s | 2.5 s |
| moved layout | 259 of 275 | 94 found, 44 derived | 89.8 s | 72.5 s |

The 16 files touched (8 edited, 8 new) miss the memo, as they should. But the move also re-derives 44
components, a third of the cross part, though no answer changes. Under 0083's symbol keys the moved definitions
keep their keys, so this move would be a delta of spans and paths only.

**Not yet measured.**
- **Cold work of the moved layout.** It needs a prime of about 130 s CPU, which is 5 minutes at this load. It is
  the next long run. It is to be compared with the prime's 128.8 s.
- **The pair without typeshed.** 0083 asks for runs with typeshed and without it.
- **Both memos off.** A cold `calls_cx` run with the per-file memo off as well as the cross memo grew past 10 GB
  RSS in 6.8 min of CPU, and was stopped. The memoised prime peaks at 842 MB. This is O135. Step 0's
  no-memo comparison waits on it.

## Reproducing

```sh
# Build the moved corpus and its map (a dev tool; Python reads only the corpus).
python3 -I benchmark/layout-invariance/move.py <corpus> <out>/dl-moved <jedi-recording.json> 8
# Run the benchmark test twice, once per layout, with RERAIL_PYTHON_CALLS_ALL=<file>, then:
python3 -I benchmark/layout-invariance/compare.py <out>/move-map.tsv original.tsv moved.tsv
```
