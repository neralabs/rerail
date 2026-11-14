# 0178. A cold run's whole-program passes run on the pool

Status: built (2026-11-13). Follows decision 0177. The pool is decision 0098's (`threads`, `rerail_accel::exec`).

## Intro

A cold batch is a chain of steps, each needing the one before:

```
phase 0 → read the 44 programs → the bridged typing run → phase 1 → prepare → local → cross
```

The rest of the CPU sat idle while the main thread ran that chain. On `dl1`, the main thread did 3.2 of the
process's 3.5 s of CPU, and the four pool threads did 0.15 s.

Most of the main thread's work is passes over the whole program, and each pass loops over relations or rules:
- the file-local split, with its fixpoints;
- simplification's congruence;
- hiding's analysis;
- reading and lowering each program;
- the inlining choice's estimates.

Inside a pass, each relation's or rule's work is independent. Those loops now run on the pool, and every result is
the one a single thread gives.

## Decision

Each pass computes its items in parallel and keeps the order-dependent part (numbering, interning, applying edits) on
the caller, in the same order as before.

| pass | what runs on the pool | why the result is the same |
|---|---|---|
| `simplify::congruence` | each relation's first signature, abstracted rules, canonical texts, and each round's signatures | classes and templates are numbered in `cands`' order afterwards |
| `file_local::split_in`: `local` | each round's rule checks | the rounds were already rounds over one state (their causes stay the same) |
| `file_local::split_in`: locations, filters, `split_constants` | each round's relations, against the state as the round began | greatest fixpoints: relations only leave, so any order reaches the same one |
| `split_memo::keys` | each relation's digest | a map |
| `hiding::hide` | each round's `analyse`, `carry`'s two fixpoints in rounds, the final bounding of each rule | a round reads its start; `carry`'s fixpoints are monotone; each rule is bounded alone |
| `combine_read` | each program's `read_text` and lowering | the library files each read are noted on the caller in program order (`rr::recorded_on_worker`, `rr::replay`; decision 0094 §6), and the first error in program order is returned |
| `keeps::estimate` | each level's rules planned | see below |

**The estimate's levels.** A relation's level is one more than the levels of the relations before it in
evaluation order that its rules read. A relation that a rule reads before it is estimated (inside a recursive
component) is given a level after that rule's. Each level's plans read the estimates of the levels before it, and
each level is then estimated in order. So every plan reads exactly the estimates the sequential order gave it. The
model's one interior cache, `Stats::steps` (a function of the tree's statistics), is now behind a mutex.

**The cost statistics are read beside the run.** `costs::start` reads the stored samples on a thread of its own.
`costs::ready` waits for that read, called from the executors' first use and from `sql_plans::estimate`, so the
read still finishes before the first wave (decision 0104). The read itself was slow because it found the 7 lanes
and forms with a `DISTINCT` over all 500,638 of the machine's index entries, which took 0.03–0.22 s. A skip scan
down `samples_by_form` now finds them in 0.3 ms.

**The CLI's main thread is one of the pool's.** The addon's load constructor (`install_tree_sitter_allocator`)
made the pool before `main` ran, because the CLI is built without `headless`. So `main`'s
`threads::configure(true)` did nothing. The main thread's parallel loops queued their work and slept, and its waits
ran no jobs. The constructor now leaves the pool to the CLI (an executable named `rerail-engine…`).

## Measured

`dl1`, cold (`RERAIL_TAPE_CACHE=off`), on `--profile profiling` builds, run in alternation with the build before
this decision, at load 32–42:

| | before | after |
|---|---|---|
| wall, 5 runs | 1.88–2.13 s (median 1.93) | 1.68–1.82 s (median 1.69) |
| user | 1.59–1.67 s | 1.71–1.80 s |
| instructions | 27.6–27.9 G | 28.2–28.5 G |
| main thread CPU (profiled) | 2.5–3.3 s | 1.7–2.2 s |

The findings are identical. The decisions in the profile ("inline or store" among them) are identical, timings
aside. `RERAIL_SPLIT_CHECK=1`, which compares every incremental split with a whole one, passes on `dl1`.

## What stays serial

- The chain of steps itself. Phase 0's graphs feed `program_texts`. The bridged rows and phase exports feed
  prepare's keys (`under_phased`), so prepare cannot start before those runs end.
- Magic adornment's greedy trials (`magic_adorn`): each trial stratifies the whole program again and depends on the
  trial before. The fix is an incremental stratification check, not more threads.
- The sequential parts of the passes: building the sets of relation names (`BTreeSet<String>`, mostly `memcmp`),
  `RuleIndex::of`, prune, lowering the program.
- Within evaluation: `split_negations`, `analyze` and what is left of the inlining choice.
