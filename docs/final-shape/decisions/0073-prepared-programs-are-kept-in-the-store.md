# 0073: A prepared program is kept in the store

Status: built (2026-11-07); its entry's form superseded by decision 0074 (facts and a tape, not JSON)

## Intro

Before a run evaluates a rule program, it prepares it. Preparing prunes the rules no output reads, simplifies their
bodies, computes negated tests on demand, lowers the program for the dialect, pushes file-local joins into the
chunks, and splits the program into its file-local and cross-file parts. Decision 0060 kept the result for the
process only. Every new process prepared each rule again, and running a pack means one process per rule or one
preparation per rule.

Decision 0062 made a rule file's tape the thing lowering reads, and named the next step: keep the lowered program
across processes, memoised per source. This record does that. A prepared program is stored in the content store's
facts memo, under its program text's stored source, by the same mechanism that keeps a file's rows (decision 0052).
A later process reads it instead of preparing again.

```
program text ──► source (sha256) ──► facts memo entry (plan: build, grammar, dialect, additions)
                                          │
        run ──► in memory? ──► in the store? ──► atoms still the store's? ──► use it
                    │ no            │ no
                    └──── prepare ──┴──► keep in memory and in the store
```

## Decision

### 1. The entry

`logic::plan::corpus::prepared` keeps a prepared program as one facts-memo entry:

- **Source:** the program texts, joined, as a stored source (`memo::source`), deduplicated by sha256.
- **Plan:** a digest of the engine build, the grammar version and the prepared key. The prepared key already names
  the texts, the dialect, and what the run added to the program (its facts, inputs and outputs).
- **Schema:** `SCHEMA = 1`, bumped when the entry's meaning changes.
- **Path:** none. The entry reads no path (`path_dependent: false`), so one entry serves every root.
- **Bytes:** the prepared program as JSON (serde derives on `front::Program` and the types it holds). The store
  compresses it with zstd as it does every memo entry.

An entry that no longer reads (another schema, a matcher that no longer builds) is a miss, and the run prepares
again and stores over it.

### 2. Its atoms are checked as decision 0060 checks them

Lowering turns strings into atoms of the store. A stored program is only valid while those strings are still those
atoms. `interned` lists every intern call lowering made. Before a run uses an entry, from memory or from the store,
it asks the store each call again and compares. Atoms are never given out again (decision 0049), so in the tape
cache's store the check passes. A run on another store (its own scratch store, a session's) fails it, and prepares.

### 3. Text tests are kept as their specs

A prepared program's text tests (decision 0037) hold compiled matchers, which are not data. `Vocabulary` now keeps
each test's operator and patterns beside its matcher (`specs`). The entry stores the specs, and reading it makes the
matchers again (`Vocabulary::from_specs`).

### 4. Who keeps it

- **Uses the store:** a run with the tape cache (`tape-cache` feature, `RERAIL_TAPE_CACHE` not `off`) that
  reproduces no T facts and is not in a session.
- **Memory only:** a session keeps it in memory, where it lives as long as the session. A T reproduction keeps it in
  memory too.
- **No store:** the browser build has no store.

### 5. The run says where it came from

`Report::prepared` is `prepared`, `memory` or `store`. `rerail-engine logic` prints it when it is not `prepared`,
and the profile's "program prepared" decision says the same.

## Measured

On 18 Python pack policies (every 25th, sorted, without b005), each `run-rr` was a fresh process over
`fixtures/python/calls/project`. CPU is user plus system, at load 42 to 51:

| pass | stats phase (holds preparing) | process CPU |
|---|---|---|
| 1: prepared and stored | 0.94 s | 5.80 s |
| 2: read from the store | 0.49 s | 4.56 s |
| 3: read from the store | 0.49 s | 4.44 s |

That is about 25 ms per process, half the stats phase. Pass 1's process CPU also refills the per-file memo for a new
build, so not all of the difference in its total is preparing. In one profiled run (`ruff-up006`), preparing was 38
ms: lowering 25 ms, and pruning, simplifying and demand 13 ms. Entries average about 500 KB of JSON, which
compresses to about 31 KB. The largest was 1 MB.

The converted-rules replay (`converted_rules_give_the_recorded_findings`) ran twice on one fresh store. Both runs
gave the recorded findings (3988 303 298 0). The first, preparing and storing, took 9.96 s user + 2.54 s sys. The
second, reading every program from the store, took 7.95 s user + 2.52 s sys (load 42). The store then held 177
entries: 31.8 MB of JSON, 1.9 MB compressed.

Most of a fresh process's CPU is still outside the phases. Over the 20 runs measured before this change, about 1.9
s of 3.6 s was spent outside every phase: starting the process, opening the store, recording the run. That cost is
not this record's and is not attributed here.

## What this does not do yet

- **The entry is JSON, not facts.** Decision 0074 replaces it: the entry is facts, kept as a `.rr` document with
  its tape, which a rule can read.
- **Matchers are compiled per process.** Specs are kept, not the compiled automata. Decision 0075 keeps each group's
  DFA in the store.
- **A pack is still prepared one rule at a time.** The Python pack does not combine into one program (O111).

## Prior work consulted

- **Lessons ledger 25:** restoring a stored derivation beats redoing it (20 to 26x for tapes). A prepared program is
  restored the same way.
- **ast-index-spike history index L919-920:** key memoised analysis by what it is a function of, and make the
  language part of every key. The plan names the build, the grammar version and the dialect, and the source names
  the text.
- **ast-index-spike history index L417 (DH L2185-2189):** "storing data in memory... is dishonest". Decision 0060's
  prepared cache was per process; the store now holds it.
- **ast-index-spike history index L460 (DH L3187-3191, L3319-3343):** persisted planning state is re-derived or
  refused on a fingerprint mismatch when a store is attached. Here the interned atoms are the fingerprint, and a
  mismatch prepares again.
- **ast-index-spike design index S112 (the compiled artifact):** a compiled query is an artifact of its own, kept
  apart from the run that uses it.
- **Decisions 0052, 0060 and 0062:** the facts memo, the in-process prepared cache and its atom check, and the
  step this record takes.
