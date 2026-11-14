# Sibling workstreams after the input-lifecycle changes

## Merge boundary

This survey follows the production input-lifecycle commit `beaf99ca9`.
The selected integration tip is `b4f0aa04b`, on `jeremybarnes/final-shape-serverplan-quality` in `Final-shape-quality`.
It was the committed local tip at the start. The fetched remote tip was `3ba1008cf`, its parent.
The merge uses the committed local tip. It does not include another worktree's staged or unstaged changes.
At inspection, the hub had an unfinished merge from executor commit `72a3a658b`.

The integration changes include default definition-based IDs, explicit node-order relations, recursive levels maintenance, and the extracted tier-1 compiler.
Merge resolution preserves the new numbering for ordinary and guest documents, plus our direct fact admission and lazy projection columns.
Granule hashing finds subtree bounds from ordered core IDs. It does not force a projection-size array to materialize.
The typed native start specification keeps its `Hash` implementation and the integration's level-copy semantics.
Both branches' projection tests remain present.

The earlier 25% CPU result describes `beaf99ca9` against its earlier baseline.
It is not a performance measurement of this merge: definition-based IDs are now the default.

## Evidence boundary

The survey reads branch history, design notes, and current working diffs.
A dirty worktree shows active implementation, not a tested or accepted result.
`benchmark/async-family-integration/merge-b4f0aa04b/worktree-survey.json` records the observed tips, status paths, and recent commits.
Worktrees can change after this snapshot. No sibling working files were modified.

## Work directly related to this branch

| Worktree | Observed work | State relative to the selected integration tip | Implication |
|---|---|---|---|
| `More-ergonomic-rule-syntax` | SQL/store waits during execution waves, direct reads of held native outputs, write-back and cache architecture | Committed tip `d3ef2ab8f`, not included | Reuse its profiler hooks. They report hazards during waves, not a complete count of SQL before and after waves. |
| `final-shape-rr-align` | Decision 0103 and T33: shared stored frames, relation identity, observer facts, maintained state across processes | Plan at `b18914ed6`; active changes introduce `content_store/frames.rs` and move kept strata | Align any retained facts, digest identity, and statistics work with this storage model. Most T33 moves remain a plan. |
| `Bend-language-investigation` | Decision 0103: `.cache output by key`; file-local and cross-file caches, typed phase state, demand-driven searches | Decision at `0d8ba511c`; active changes include `cache_rows.rs`, `cache_store.rs`, and tests | The cache semantics and the shared frame storage need to meet. Do not build another retention mechanism here. |
| `rer-131-tapes-store-only-named-nodes-anonymous-t` | Compact tape/node representation, token semantics, direct omission of implied tokens during encoding | T30 at `00482ed0b`; substantial active changes move toward format 14 and change the earlier token-recognition adapters | This overlaps directly with our validator, facts, projection, and digest contracts. It is not ready to import from the working tree. |
| `wolfeel` | One executor pool, CPU/GPU scheduling, late work, chained submissions; current work puts regex prefilters through executor launches | Tip `87396ebb3`; additional dirty changes. The hub's pending merge targets `72a3a658b` | Reuse its executor for admission work. Do not introduce another worker pool. |
| `Typing-support` | Stable node IDs per definition/granule, across languages and guests; explicit order for consumers that need it | Tip `7533ab721` is included | This changes our default input cost and ID contract. Preserve stable IDs when reducing projection work. |
| `Two-pass-roots-and-cross-file` | Recursive maintenance by levels, delta-size choice between maintenance and full derivation, fewer planner passes | Most work included; three later commits through `5fd2ed437` and active planner edits remain outside | Avoid independent changes to maintenance thresholds, retained identities, or recursive scheduling. |
| `Matrix-offload-spikes` | Closure decomposition, dependency-level batches, acceleration placement, prepared-program storage | Tip `4a7c6c945`, not fully included | Owns bulk relational execution and device work. Its measurements use different workloads from our async family. |

### Important qualifications

RER-131's T30 note reports fewer held nodes, but its CPU and peak-memory comparison is not controlled.
The note explicitly leaves those costs unexplained and requests a matched baseline.
Its live diff changes the encoding and removes parts of the earlier recognition design.
The committed note is therefore not a complete description of its current implementation.

The two cache workstreams share a direction but currently introduce different storage pieces: `frames` and `cache_rows`.
Their integration needs to settle the physical storage contract as well as the `.cache` semantics.
The inspected frame read loops over requested relations, and the cache-hit read loops over requested keys, with SQL inside those loops.
A relation-based cache design alone does not ensure bulk storage execution.

## Other engine work

| Worktree | Current focus | State |
|---|---|---|
| `Parser-generators` | Extract tier-1, tier-2, parser, and compiler machinery into crates | Tier-1 extraction included; later extraction commits through `3e872d7f5` remain outside |
| `Rerail-rule-expressivity` | Retained SQL regions across native producer gates; component scheduling versus a SQL version loop | Commits through `df4ec32d6`, with active loop-scheduling changes |
| `Current-state-benchmarks` | One-file edit benchmark, session/reopen costs, memo volume, narrow rule demands | Clean tip `72f7518d7`; the harness is built, while its T31 workplan contains additional proposals |
| `Data-layer-rule-expressivity` | Exact supplied dispatch facts, report contracts, typed Python primitive values | Commits through `e7687ee69`, with active acceptance and rule-port changes |
| `Disk-space-controller` | Rust build disk guard | Active scripts, configuration, and tests |
| `Transducer-demos` | Visualizations and interactive demos | Active untracked demo work |
| `Tetris` | Browser/WASM demonstration | Clean tip, latest commit November 8 |
| `skimmer` | Public-repository audit/release work | Clean tip, latest commit November 4 |

The literal `Pulling-it-all-together`, original SQL-capture branch, NER-1562, and `Shared-structural-execution` worktrees are clean.
Their latest commits are from October 26. They provide prior work, not evidence of a current parallel implementation.

## What remains for this branch

The singleton tape-admission path remains in all six relevant working copies inspected:
`Final-shape-quality`, `wolfeel`, `final-shape-rr-align`, `Bend-language-investigation`, `More-ergonomic-rule-syntax`, and RER-131.
Each calls `tape_by_source` for one file, followed by `get_tapes_bulk(&[id])` for one tape.
No concrete replacement for that path appeared in the inspected changes.
T33 plans to express tape requests as facts, so the physical bulk operation must fit that future request interface.

The next bounded implementation can therefore be bulk tape admission:

- Resolve tape identities for the existing input window, then fetch its frames together.
- Decode and validate through the existing execution pool, outside the store lock.
- Preserve the source, grammar, format, identity, and corruption contracts.
- Use the existing SQL-wait profiling work when integrated, and add full-job store execution totals.
- Keep the physical batch API usable by the shared fact/request model. Add no retained cache or private pool.

Demanded line statistics still need work, but their ownership overlaps T33's statistics migration.
Index omission and deeper tape/fact layout changes need to follow RER-131's settled representation.
Digest elimination must preserve the relation-identity and invalidation contracts under active development.


## Merge validation

The optimized build passes. All 110 focused test executions pass.
These cover core columns, tape admission, projection and granule IDs, lazy arrays, guests, spans, compiler composition, profiling, and recursive levels.
The async positive fixture passes in default definition-ID mode and explicit preorder mode.
All four edge fixtures retain their 178 finding keys.
The full 5,725-file workload retains zero findings, 81 rule executions, and 785,573 VM instructions.
The new granule reader leaves projection sizes lazy: only 35 documents demand them, as before the merge.
Only 25 documents demand depths, and 34 demand token spans.

This is merge validation, not a new speedup comparison.
The raw reports, build log, test logs, driver, and source hashes are recorded beside the survey snapshot.
