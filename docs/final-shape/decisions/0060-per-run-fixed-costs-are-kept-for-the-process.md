# 0060: A run's fixed costs are kept for the process; collection decides only when it can change something

Status: proposed (2026-11-07)

## Intro

The replay of the recorded findings (`converted_rules_give_the_recorded_findings`) runs 303 corpus runs. Each run
covers a few tiny files. It took 41.5 s user + 10.3 s sys CPU (at a load of 22), or about 170 ms per run. The
engine's own profile spans covered only 13.7 s of that. A whole-process sample (`sample`, on-CPU stacks only)
showed where the rest went:

| share | what |
|---|---|
| 33% | the test harness compiling the same rule file again for each event, building every regex and glob matcher |
| 29% | store collection after every run: a keep decision over every manifest, then a sweep of the store |
| 25% | preparing the program: parsing, guest hiding, prune, simplify, demand, lowering, pushing joins, the split |
| <10% | running the rules |

Every one of these is a fixed cost per run, and none depends on the files. The spike found the same thing (F3:
"per-file fixed cost, repeated processing"). This record keeps each cost for as long as its inputs are unchanged.
It also stops collection from running when it can change nothing.

## Decision

### 1. Collection decides only when a decision can change something

A new manifest still makes collection due (`corpus_state.due = 1`). A pass now first asks
`corpus_decision_worth`, and decides only if one of these holds:

- a root recorded since the last decision has more manifests than its protected runs (history grows only this
  way);
- `decide_every_runs` runs (256) have passed since the last decision;
- `decide_every_secs` seconds (600) have passed (needed for the backstop and for gone roots).

If none holds, the pass clears `due`, and neither decides nor sweeps. A new root's first runs are protected, so a
series of fresh roots (tests, the replay, temporary checkouts) rarely decides. `corpus_decided` (additive) records
the run counter and time of the last decision. A run starts its collector thread only when `corpus_pass_wanted`
says so. The workspace watcher asks the same question.

### 2. A root whose directory is gone keeps nothing

At a decision, a root that has been idle for `missing_secs` (1 h) and whose path no longer exists is left out. Its
manifests are evicted, and its stat index and row are deleted. No run can be warm there again. The shared tape cache
held 31,287 manifests from the temporary roots of past runs. The decision read every one of them, after every run.

### 3. Values a run derives from text are kept for the process

Each value is keyed by everything it is a function of (history index: "key memoized analysis by what it is a
function of… never by object identity"; "language must be part of every cache key").

- **`rr::read`:** each file's statements, by path and content digest. Programs that include the same library parse
  it once.
- **`corpus::combine`:** the combined program (parsing plus guest hiding), by the digest of its texts. It reads no
  file. Up to 32 are kept.
- **The prepared program (`Prepared`):** prune, simplify, demand, vocabulary, lowering, nearest sets, pushed joins,
  location and the split. The key is the digest of the texts, the run's additions to the program (`facts`, `inputs`,
  `outputs`), the stage and the dialect.
  - The lowered program contains the store's atom ids. On reuse, every intern call that lowering made is repeated
    against the store, and the cached copy is used only if the store returns the same atoms. A different or
    recreated store prepares the program again.
  - The vocabulary's run state is fresh each time (`Vocabulary::again`).
  - The profile's decisions are printed as before, plus "program prepared: reused" on a hit. Up to 16 are kept.

### 4. The run's scratch databases do not sync

`store.sqlite` (synchronous off) and `cross.sqlite` (journal in memory, synchronous off) are deleted by every run.
The export tables are made in one transaction, not one commit each.

### 5. The replay harness compiles each rule once

The converter's compiled rule is kept by its text, and a hand-written rule's printed program by its check.

## Measurements

Replay, 3988 events, 303 runs, 298 matching, 0 errors throughout. CPU is user + sys.

| step | user | sys | load |
|---|---|---|---|
| before | 41.5 s | 10.3 s | 22 |
| 1, 2, 5 (compiled rules), `combine` kept | 13.4 s | 3.3 s | 75 |
| `rr::read` kept, scratch databases, printed rules | 12.2 s | 2.0 s | 110 |
| prepared program kept | 7.3 s | 1.9 s | 87 |

The first run after this change pays a one-time decision that evicts the gone roots. A run that reaches
`decide_every_secs` pays one decision and sweep (18.6 s user, measured once).

## Not done

- **Guest hiding as bounds or views.** Its cost was in parsing and hiding, which `combine` now does once per
  program. The copied `rr_in_*` relations no longer show in the profile.
- **The converter reading the rule without building its matchers.** It now compiles each rule once per process, and
  no longer shows in the profile. The CLI converts once per invocation anyway.

## Prior work consulted

- **ast-index-spike, design index L1131-1136 (LA 6 findings, F3):** "per-file fixed cost, repeated processing,
  all-or-nothing cache invalidation". The costs here are per run, and each cache is invalidated by its own inputs
  only.
- **History index L919-920:** key by what a value is a function of, never by object identity, and make the language
  part of every key. The prepared program's key includes the dialect, and its atoms are checked against the store
  rather than assumed.
- **History index L668 (`nera_re` caches compiled regex via auxdata):** compile a matcher once, not once per use.
  This applies the same rule to the harness's compiled rules.
- **Lessons ledger 25:** restoring a cached derivation beats redoing it (20–26x for tapes). The same holds for
  derived programs.
- **03 §14, decision 0053:** collection is bounded by size with a backstop. This record keeps that policy and
  changes only when a decision runs.

## Departs from

- **0053's "a new manifest makes a decision due".** It still marks the store due, but the decision is skipped when
  it cannot evict anything.
