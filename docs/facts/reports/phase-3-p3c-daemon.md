# Phase 3, P3c: store and daemon

Branch `Moti-Velz/facts-engine-p3c-daemon` (nested worktree `.worktrees/p3c-daemon`), from integration `1c5a84b7a`. Run directories: `~/rerail-bench/facts/phase-3/p3c/runs/`. Counts first; every wall-clock below is uncalibrated (loaded machine, 18 cores, other slices running).

## Gate rows

| Id | Target | Measured | Result | Run dir |
|---|---|---|---|---|
| G3c.1 unchanged re-apply | 0 work | 40 re-syncs of the same commit: 0 changed paths, 0 clauses evaluated; 224,925 per-file re-edits by Git blob id (every tracked `.py`/`.rs` of the 40 worktrees): 0 clauses evaluated, 0 memo lookups, 0 `need_bytes`, 0 changed paths; p50 127 ms per worktree for all 5,600 requests | **pass** | `20261023T142823-facts-first40` |
| G3c.1 new worktree | ≤ 1 s | 39 worktrees after the first, synced from the Git ODB over the warm store: p50 2.2 s, p95 8.7 s, max 9.0 s; the cost is evaluating each worktree's new contents (p50 11 pairs not seen before, 2–74), not membership. A worktree whose contents are known (clone, or 30: 2 new pairs) syncs in 0.27–0.5 s | **fail** (the work is evaluation of new contents; see § Open) | same |
| G3c.1 per-file findings after an edit | p95 ≤ 100 ms | 800 harness edits (20 per worktree; comment appended to the minimum, median, p95 and maximum-size Python file in turn, hash first): all p50 2.0 ms, p95 2,469 ms. By bucket p95: minimum 3.5, median 12.5, p95-size 27.1, maximum 3,308 ms. Edits that evaluated (180; the others' content was known from another worktree): p50 12.3 ms, p95 3,405 ms; without the maximum-size bucket p95 48.1 ms | **fail** at the harness's mix (the 718 KB file costs seconds); pass for every bucket but the maximum | same |
| G3c.2 resident, 40 worktrees, steady state | < 1 GiB | **896 MiB** RSS after 40 syncs, 800 edits, rebalance (live heap 629 MB) **without the cross-file model**. With the from-scratch stand-in (`--cross scratch`, worktree 01 only): 1,447 MiB while it holds one worktree's stored rows for `eval::finish`; P3a's model is not merged, so no figure with it | **pass as a floor** (without P3a's model) | same; `20261023T143525-facts-cross-scratch2` |
| G3c.3 per-file findings equal native's | 0 differences, every worktree | not measured: the native 40-worktree run with a shared store (`--store-path`) died on worktree 01 (`stored results for evicted _internal/pure_lazy.py are unavailable; apply it again`, native's store path, not the daemon's); relaunched with a store per workspace (> 15 min per worktree on this machine, hours for 40). Per-workspace findings land in its `findings/` as each finishes; then `just rerail-workspaces compare-findings <native>/findings <facts>/findings <facts>/facts.json` | **open** | native: `20261023T144215-native-first40-ownstore` (running); facts: `20261023T142823-facts-first40/findings` |
| G3c.3 cross-file equal to from-scratch | for the clauses P3a's model runs | P3a's model is not plugged in; the daemon's cross-file side is the from-scratch phase itself | **n/a** until P3a reports | — |
| Store migration tests | green | 6 `content_store::facts_memo::tests` + the 68 existing `content_store::` tests: 74 passed | pass | `cargo test --profile loop -p rerail-engine --lib content_store::` |
| T0 / T1 | green | T0 3/3 checks, 3/3 mutants (12–73 s with builds); T1 sub200 90/90, memo 14,982/14,982 warm and 0/14,982 cold with every clause's counts equal, ref50 0 mismatches | pass | `~/rerail-bench/facts/loop/runs/20261023T1409*-t1` and later |

Steady state of the 40-worktree run: 7,257 contents seen, 7,103 held; 7,163 held (path, content) pairs with 12,606 per-file findings resident; base 5,582 paths, 8 cluster bases (12/4/5/2/5/2/7/3 worktrees), worktree deltas median 17 paths (max 187); 12,255 tag records; rebalance 4.4 s (cost 6,673 entries vs 26,391 flat). Store 205 MB. The first worktree: 16.1 s (5,688 blobs read, 1,045,546 clause runs evaluated, 4,587 served by content-pure entries of identical content at other paths).

## What landed

1. **Store** (`native/src/content_store/facts_memo.rs`, `schema.rs`, `reclamation.rs`, `mod.rs`): `facts_results` (one row per clause run with something to report: findings, failures, repository rows, budget rows), `facts_completions` (per (content, fact schema, path, plan set) completion words, bit i = clause i ran with nothing to report, with those clauses' counts in one frame), `facts_plan_sets` (a set's ordinals → plan digests, so a set that changed a few plans reuses the others by digest). Key: `(source, fact schema, path or '' for content-pure, plan digest)`; the content is a stored source found by its blake3 alias (`HashKey::blake3`, algorithm 4). Every row cascades from `source_objects` and references no other shared row, so collection by an older binary cannot orphan or misread a row: the tables are additive and **the floor stays 1** (`content-v1.sqlite`), per schema.rs's rule; the new binary's `collect` drops plan sets no completion names. Pins in bulk (`pin_facts_sources`/`unpin_facts_sources`) keep held contents alive.
2. **Memo port** (`spikes/facts-a/src/memo.rs`, `repo.rs`): `memo::Memo` over the store (`FACTS_STORE`, else `FACTS_MEMO_DIR/content-v1.sqlite`); the plan digest, `ENGINE_VERSION` and the format string are unchanged (the memo key is fixed). **Content-pure split**: `eval::Rule::content_pure` (with `ast::reads_path`, `lowered::Plan::reads_path`) admits a clause whose inputs have no `paths` filter, path projection or `filePath` term, a regex clause on content without `paths`, a budget not grouping or filtering on the path column, and repository relational clauses; per-file relational clauses stay path-keyed (their stages may read `__path`). A pure entry elides the path (findings, failures, budget rows, the repository rows' first column) and is filled in on read; a path anywhere else in a pure entry panics (tripwire, never hit on sub200 or the 40 worktrees).
3. **Daemon** (`spikes/facts-a/src/daemon/`, `facts-a daemon`): one process, Unix socket, framed JSON + bytes (`docs/facts/PROTOCOL.md`); `sync` from the Git ODB (commit or index snapshot; blobs the store knows by Git id are not read), `edit` hash first (blake3 or Git blob id; `need_bytes` then bytes), per-file findings answered synchronously as epoch-stamped deltas, cross-file pushed when idle with the epoch. Membership: Spike B's `membership.rs` copied (`daemon/membership.rs`), per-path plurality base, cluster bases by the exact DP, worktree deltas; `Resident` reference-counts held pairs; pairs released drop their results and unpin their contents. Evaluation fans pairs over `--threads`, and a pair's rules over the threads left.
4. **Cross-file interface** (`daemon/crossfile.rs`): `trait CrossFile { fn apply(&mut self, worktree, members, delta) -> FindingsDelta }`; per-file = pattern clauses and per-file relational clauses of rules without `report_when` (529 of 558 clauses); cross-file = budgets, repository relational clauses, every clause of a `report_when` rule (29). `FromScratch` (`--cross scratch`) runs `eval::finish` over every member's stored rows: the reference for P3a's model.
5. **Harness** (`benchmark/workspaces/{workspaces,facts}.ts`, `conformance/data-layer/facts-daemon.ts`): the 47 workspaces table, `--first N`, `setup --first N`, `--findings-out DIR` (native and facts), `run --engine facts --daemon <facts-a>` and `compare-findings`.

Location decision: the daemon is a module of `facts-a`, not a `facts/daemon` crate: the per-file engine (`eval`, `facts`, `memo`) lives in facts-a, a binary crate; a library split would touch every module the other slices edit.

## How measured

```
just rerail-workspaces setup --data-layer ~/repos/nera-data-layer --workspaces ~/rerail-bench/memory47/workspaces --first 40
cargo build --profile loop -p facts-a --features facts-a/mutants
just rerail-workspaces run --engine facts --daemon target/loop/facts-a --rules packs/data-layer/candidates/rules \
  --workspaces ~/rerail-bench/memory47/workspaces --first 40 --edits 20 --threads 16 --store-path <run>/store.sqlite \
  --output <run>/facts.json --findings-out <run>/findings
just rerail-workspaces run --engine native --rules packs/data-layer/candidates/rules --workspaces … --first 40 --edits 20 \
  --idle-ms 0 --store-path <run>/store.sqlite --output <run>/native.json --findings-out <run>/findings --addon target-addon/addon/rerail_engine.darwin-arm64.node
just rerail-workspaces compare-findings <native run>/findings <facts run>/findings <facts run>/facts.json
```

The 60 rules: the daemon compiles 58; 2 are refused by name at compile (`imperative_private_functions`: `calls` rule key; `internal_dead_code`: projection `file.module`), reported in `hello` and in every `findings` reply's failures.

## Interface changes

- `rerail_engine::content_store`: `FactsPlanSet`, `FactsMemoKey`, `FactsResult`, `FactsCompletion`, `FactsMemo`; `ContentStore::{facts_source, put_facts_source, facts_memo, put_facts_memo, pin_facts_sources, unpin_facts_sources, facts_generation}`; `HashKey::blake3`.
- facts-a: `memo::dir` → `memo::store_path` + `memo::Memo` (`file`, `file_at`, `encode`), `FileMemo::{get_reported, put_encoded}`; `eval::Rule::content_pure`; `ast::reads_path`; `lowered::Plan::reads_path`; `repo::in_scope` is `pub(crate)`; new `facts-a daemon`.
- `FACTS_MEMO_DIR` now names the directory of the memo's store file (`content-v1.sqlite`); the old JSON memo directory (`s1/`) is no longer read.

## Open

1. **G3c.1 latency is evaluation-bound.** New worktrees and edits that change a large file pay one file's 60-rule evaluation; on the 718 KB `sql_connection.py` that is 2.7–3.3 s even with the rules spread over 16 threads (one rule dominates). The daemon adds ≈ 1 ms (memo-served edits p50 1–2 ms). Levers: per-rule cost on large files (P3b's bounds and the per-file engine), incremental per-file evaluation by region, or answering the cheap clauses first and the slow ones as a second delta. The 40-worktree run used the binary before the pair/rule thread split (`6e85b3d61`): with few new pairs, each pair now gets threads/pairs workers; not re-measured.
2. **Resident 896 MiB is mostly not state.** The held state is small (7k pairs, 12.6k findings, 40 deltas); the live heap is 629 MB (compiled rules ≈ 130 MB RSS at start; the rest not attributed: candidates are the regex cache, thread-local evaluation caches on the engine thread and the store's read caches). RSS above heap is allocator retention after the first worktree's cold evaluation. Attribution with `FACTS_MEMTRACE`-style heap probes is the next step before adding P3a's model.
3. **G3c.3**: run `compare-findings` once the native run has written its workspaces (it writes `NN.scan.json`/`NN.edits.json` per workspace as it goes). Native with a shared `--store-path` fails on this rule set (above); that is a native-store issue to report separately.
4. **Cross-file**: P3a's model (facts-b `gen/hybrid.rs`) plugs in behind `CrossFile` once facts-b exposes a lib target (P3a's stated preference); its `apply(worktree, [(path, old hash?, new hash?)])` maps 1:1 onto `PathChange`. `FromScratch` costs one repository phase per push (worktree 01: ≈ 56 s), and it runs on the engine thread: in the `--cross scratch` run every edit waited ≈ 56 s behind the previous push (edits p50 56.3 s). Before any real model goes in, the cross-file side must run off the engine thread (a worker over membership snapshots, pushes stamped with the snapshot's epoch) so per-file replies never wait for it.
5. **Warm restart**: a fresh daemon over the 40-run's store synced worktree 01 in 486 ms with 1,050,133 clause runs served by the memo and 0 evaluated (`20261023T143525-facts-cross-scratch2`): a new worktree whose contents the store knows meets the 1 s target; the G3c.1 misses are new contents.

## Commits

- `497202f75` p3c: facts memo in the content store; content-pure clauses; facts-a repo on it.
- `37cc0b22f` p3c: facts daemon, protocol, Git ODB sync, membership, CrossFile, harness `--engine facts`, PROTOCOL.md.
- `6e85b3d61` p3c: compare-findings, pair/rule thread split.
- this report.

## Resume notes (first pass; superseded by the continuation's, last)

- Runs: facts 40 `~/rerail-bench/facts/phase-3/p3c/runs/20261023T142823-facts-first40` (`facts.json`, `findings/NN.{scan,edits}.json`, `store.sqlite`); native 40 running `…/20261023T144215-native-first40-ownstore` (the shared-store attempt `…/20261023T143432-native-first40` died); from-scratch cross-file run `…/20261023T143525-facts-cross-scratch2` (stopped after worktree 01); repo baseline of worktree 01 with all 60 checks `…/20261023T141318-repo01-all60` (per-file costs: p50 4.1 ms, p95 58.5 ms, max 4.1 s single-threaded).
- Worktrees: `~/rerail-bench/memory47/workspaces/01…40` (+47) are git worktrees of `~/repos/nera-data-layer` (`setup --first 40`); `~/tempo-qe/workspaces47` are plain trees whose git links are broken.
- Native addon for the harness: `target-addon/addon/rerail_engine.darwin-arm64.node` (built from this tree's `native/`, untracked).
- Next: G3c.3 compare; heap attribution (Open 2); re-measure G3c.1 with the thread split; wire P3a's model; the Spike B 1,111-event replay through the daemon (events in `~/rerail-bench/facts/spike-b/edits/seed-5eedb/`, blake3 hashes, contents in `spike-b/contents/`) is not wired yet.

## Continuation: streaming, parallel evaluation, memory, parity

Branch `Moti-Velz/facts-engine-p3c2-daemon` (worktree `.worktrees/p3c2-daemon`), from integration `4ab324fde`. Every wall-clock below is uncalibrated: the machine ran at load 20–78 throughout (five native parity processes, other slices), and the same binary's percentiles moved by 30–60% between runs; counts and component bytes do not move.

### Gate rows, re-measured

Headline run `20261023T1620-facts-first40-final16` (final binary, `--threads 16`, fresh store, 40 worktrees, 20 edits each); `…-final4` is the same at `--threads 4`; `…-stream` is the streaming binary before the persistent pool, at lower load. All under `~/rerail-bench/facts/phase-3/p3c/runs/`. "First" is the client's time to the first findings frame (a partial frame, or the reply when none comes); "complete" is to the reply.

| Id | Target | Measured | Result | Run dir |
|---|---|---|---|---|
| G3c.1 unchanged re-apply | 0 work | 40 re-syncs: 0 changed paths; 224,925 re-edits by Git blob id: 0 clauses evaluated, 0 memo lookups, 0 `need_bytes`, 0 changed paths | **pass** | `…-final16` |
| G3c.1 new worktree, first findings | ≤ 1 s | 39 worktrees after the first (p50 11 new pairs, 0–362): p50 44 ms, p95 306 ms, max 362 ms (`…-stream`: 29 / 179 / 279 ms) | **pass** | `…-final16` |
| G3c.1 new worktree, complete | reported apart | p50 2.2 s, p95 9.5 s, max 10.4 s (`…-stream`: 1.3 / 7.2 / 8.1 s; first pass: 2.2 / 8.7 / 9.0 s); the tail is the worktrees with the 718 KB file's or many new contents (§ 1) | — | `…-final16` |
| G3c.1 edit, first findings | p95 ≤ 100 ms | 800 edits: p50 3.4, p95 181, p99 226, max 267 ms. By bucket, p95: minimum 9.2, median 23.2, p95-size 31.7, maximum 224 ms; without the maximum bucket p95 24.7, p99 40.6 ms (`…-stream`: all p95 135, without the maximum 16.6 ms) | **fail** on the mix: an evaluating edit of the 718 KB file shows its first findings after its facts build (parse 68 ms + extract 9 ms single-threaded, unloaded) and its cheapest rule run, p50 198 ms under load; **pass** for the other three buckets | `…-final16` |
| G3c.1 edit, complete | reported apart | p50 3.4, p95 3,893, p99 5,581, max 7,618 ms; by bucket p95: 9.2 / 23.2 / 71.3 / 5,566 ms. Evaluating edits of the 718 KB file complete at p50 5.3 s (`…-stream` 3.2 s): the time of one clause (§ 1) | **fail** (rule shape, § 1) | `…-final16` |
| G3c.2 resident, 40 worktrees, file-only rules | < 1 GiB | 16 threads: live heap 652 MB (Rust 648 + C 3.6), jemalloc resident 772 MiB. 4 threads: 294 MB (290 + 3.6), resident 349 MiB. RSS read 305 and 387 MiB: macOS compresses pages under this load, so RSS is not a bound here; jemalloc's resident is. Attribution in § 3 | **pass as a floor**; headroom for P3a's model in § 3 | `…-final16`, `…-final4` |
| G3c.3 per-file findings equal native's | 0 differences, every worktree | 40/40 worktrees, 188,979 per-file findings on each side, **0 differences** (no check differs). Native also fails `grammar_fragment_usage/captured-return` (`source-incomplete`) on every worktree: a membership-level failure of a cross-file clause (SCRATCHPAD § `captured-return`), which the daemon's from-scratch cross side reports too | **pass** | native `20261023T1510-native-first40-scan`, facts `20261023T1545-facts-first40-pool16` |
| Streaming exactness | a client applying every frame holds the daemon's findings | every worktree of every run after its sync and after its 20 edits: 0 rows differ from `findings` (`heldMismatches`) | pass | every run above |
| T0 / T1 | green | T0: 3/3 checks (facts-a-unit 18 tests, the new `crossfile::tests` among them), no mutant site in the changed files. T1: sub200 90/90 findings, 0 failures, equal to the reference in every clause, memo 14,982/14,982; ref50 0 mismatches; bench 5 archetypes (no evaluation file changed) | pass | `~/rerail-bench/facts/loop/runs/20261023T162122-t0`, `…/20261023T162453-t1` |

### 1. Edit latency is one clause on one large file

`facts-a repo --checks <all 60> --threads 1` on `impl/common/l2200_sql_dialect/sql_connection.py` alone (718,905 bytes, 123,120 nodes, 574 function definitions, 123 named `*_async`; run `20261023T1505-attr-sqlconn`): 446 clauses of 41 rules run, 4,407 ms.

| Rule | ms | share | Dominant clause and its counts |
|---|---|---|---|
| `sync_async_signatures` (1 clause) | 2,991 | 67.9% | `transaction-connection` (forbid ast): `match_nodes` 123,120, nothing else counted |
| `serverplan_state_tripwires` (32) | 736 | 16.7% | `state-flow-1` 668 ms: 72 fixpoint rounds, 457,494 fix rows, 2,672,512 rows scanned, 1,256,330 probes |
| `grammar_fragment_usage` (5) | 145 | 3.3% | `captured-return` 140 ms, all in its input build |
| `async_sync_delegation` (13) | 123 | 2.8% | `wrapper-shape` 112 ms |
| `serverplan_quality` (7) | 117 | 2.7% | `quality-contracts-2` 65 ms, `quality-conditional-1` 42 ms |
| 36 other rules | 295 | 6.7% | none above 54 ms |

(a) **A file's clauses over the workers.** Before, one file was one worker per rule (`6e85b3d61`); now one queue of tasks over every pair of a request (`daemon/perfile.rs`): a whole pair per task when there are at least two pairs per worker, else one rule's missed clauses on one pair, split into runs when a few large files would leave workers idle (the 718 KB edit: 83 tasks over 16 workers), largest files first, each worker building a pair's facts once. Inputs are not shared across a rule's clauses on this file (each input has one consumer), so the split costs only the facts build per worker. Completion cannot go below one clause: `transaction-connection` is 68% of the file and one clause.

(b) **Streaming.** A `sync` or `edit` that evaluates answers with partial `findings` frames, epoch-stamped, then the reply (PROTOCOL.md § Streaming): first the resident and memo-served paths, then each batch of rule runs as workers finish, a clause's old results removed in the frame that introduces its new ones; the reply carries the rest. An edit of the 718 KB file sends 4–5 frames (p50); its first comes after the facts build plus the cheapest rule run, its last after `transaction-connection`. The harness applies every frame and checks the result against `findings` (0 differences, every worktree, every run).

(c) **What the dominating rule does (rule shape, for Jeremy).** `transaction-connection` matches each `function_definition` named `*_async` with a connection-typed first positional parameter, `not later-signature` (`inside: module` → `has: function_definition`, `stopBy: end`, same `$NAME`, laid out after the current one), and `inside: module` → `has: function_definition` whose name equals the async name's base, with a transaction-typed first parameter and again `not later-signature`. Each candidate definition searches the whole module, so its cost is (definitions × nodes): over the 24 in-scope files of a size-stratified sample of worktree 01 (`20261023T1505-attr-sqlconn/scale2`), time ∝ nodes^1.86 and ∝ (nodes × definitions)^0.95. P3b's census (`census/classes.md`) classes it `O(n·d)` ("relation searches over subtrees and ancestors"); the measured class is `O(n·F)`, quadratic since F grows with n. Its counts show only the 123,120 root visits: the per-candidate module searches are not counted, so the algebraic budget does not see this work either (P3b: a count for `inside`/`has` subtree visits, and the class). For the rule: "a later definition with the same name" and "a sync twin with a transaction parameter" are joins on the function name; as a relational clause (definitions grouped by name; async names minus `_async` joined to sync definitions) it is linear. This is the rule, not the daemon: no scheduling makes one 3 s clause finish in 100 ms.

### 2. New worktree

(a) The sync answers the paths whose new pair is resident (the other worktrees hold them) and every memo-served clause in its first frame, then streams the evaluated ones (9 frames p50). First findings p50 44 ms, p95 306 ms, max 362 ms against 1 s; complete reported apart (table). (b) A new worktree's new contents (p50 11, up to 362) are one task queue: with fewer than two pairs per worker, rule runs of every new pair over all workers; with more, whole pairs, largest first. (c) Measured above; the complete tail is the worktrees whose new contents include large files (§ 1).

### 3. Memory attribution

`shutdown` with `attribute: true` tears the engine down one component at a time and reports the Rust heap (jemalloc `stats.allocated`) and C heap (system malloc zones: SQLite) each drop returned; `stats.memory` reports both allocators. 40 worktrees, file-only rules (`--cross none`), steady state:

| Component | 16 threads | 4 threads | Notes |
|---|---|---|---|
| Compiled rules | 556.7 MB | 240.8 MB | 135.6 MB + **26.3 MB per evaluation thread**: the programs (1 thread, worktree 01 only: 136.6 MB) plus each thread's regex match caches. Each of the rules' `regex::Regex` clones (one per use of a pattern; 1,055 distinct patterns) keeps one match cache per thread that used it (`regex-automata` pool): ≈ 25 KB per pattern per thread |
| Process remainder (statics, thread-locals) | 86.0 MB | 44.0 MB | 30 MB (the process regex map's compiled programs, grammar) + 3.5 MB per evaluation thread (facts' and rx's thread-locals) |
| Per-file results | 0.8 MB | 0.8 MB | 7,163 held pairs, 12,606 findings |
| Membership | 2.1 MB | 2.0 MB | 12,255 tag records; base 5,582 paths; 8 cluster bases; 40 deltas |
| Interner | 2.5 MB | 2.5 MB | 5,775 paths, 7,257 contents, 82 codes |
| Git object databases | 0.2 MB | 0.2 MB | one repository |
| Memo index; content store (read caches, plan sets) | 0.0 MB | 0.0 MB | the read caches are empty between requests (30 s TTL; arenas are mmap'd) |
| SQLite (C heap, every connection) | 3.6 MB | 3.6 MB | `cache_size` 1 MiB per thread connection |
| Worker scratch (transient) | 108 MB | 290 MB | largest net heap one worker held during a request (`thread.peak`); 290 MB is a whole-pair task of the cold first sync |
| **Live heap** | **652 MB** | **294 MB** | jemalloc resident 772 / 349 MiB (pages and 24 / 15 MiB metadata) |

The first pass's unattributed heap was the regex caches, made worse by threads spawned per request: each new thread's caches stayed in the regexes' pools after it exited (`--threads 4` with threads spawned per request: compiled rules 575 MB; `…-threads4`). Evaluation now runs on one persistent pool, so the caches are bounded by `--threads`; after each evaluating request a purge thread returns the freed pages (`arena.<all>.purge`, 218 purges, max 155 ms, off the engine thread).

**Floor for 40 worktrees, file-only rules:** about 170 MB + 30 MB per evaluation thread + 6 MB of state (+ 3.6 MB C heap): 294 MB at 4 threads, 652 MB at 16. **Headroom under 1 GiB** for P3a's model, against jemalloc's resident plus the binary (44 MiB): about 630 MiB at `--threads 4`, about 200 MiB at `--threads 16`, before transient scratch. Lowering threads costs the cold first worktree (82 s at 4 threads against 25 s at 16, this load) and the complete tail, not first findings (§ table, `…-final4`: edits first p95 146 ms, new worktree first p95 173 ms). The lever after that is the per-clone caches: rules hold a clone per use of a pattern, each with its own pool; sharing one `Regex` per pattern would make the per-thread cost per pattern rather than per use.

### 4. Cross-file on its own thread

`crossfile::Worker` owns the `CrossFile` (now `CrossFile: Send`) on its own thread. After each membership change the engine sends the worktree's membership snapshot, its epoch, and the changed paths, and goes on; the thread takes everything queued, runs once per worktree with the latest snapshot and every change since its last run, and pushes the delta stamped with the snapshot's epoch. `FromScratch` is unchanged and stays the stand-in until P3a's model lands. Run `20261023T1610-facts-cross-scratch-worker` (`--cross scratch`, worktrees 01–02, 8 edits each): the edits completed in 1–4,170 ms (the 4 s ones are the 718 KB file's evaluation), where the first pass waited ≈ 56 s behind each push; pushes: 01 at epoch 1 (108 s run, 5,725 files read), 01 at epoch 10 (8 edits folded), 02 at epoch 10 (its sync and 8 edits folded, 5,753 changes). The from-scratch failures include `grammar_fragment_usage/captured-return` `source-incomplete`, native's failure. Test: `crossfile::tests::queued_changes_fold_into_one_run_per_worktree`.

### 5. G3c.3

Native ran one process per workspace with its own temporary store and `--edits 0`: with edits, native's `evictInactive` between them fails on this rule set (`stored results for evicted _internal/pure_lazy.py are unavailable; apply it again`; the relaunch `20261023T144215-native-first40-ownstore` with a store per workspace died the same way on worktree 01 after its scan). `compare-findings` mapped native's rule id `data-layer-data-layer-compat-aliases` to check `data_layer_compat_aliases` while the facts engine strips every leading `data-layer-` (`compat_aliases`); fixed (`checkOf`), it reports 0 differences on 40/40 worktrees. The daemon's edits' findings were checked against its own `findings` (0 differences), not against native's.

### Interface changes

- Protocol 2: partial `findings` frames (`partial: true`) before a `sync`/`edit` reply; the reply carries `partial: false`, `frames`, `tasks`, `workers`, `scratch_peak`; `stats.memory`; `shutdown` takes `attribute`; pushes carry `members` and `changes`.
- `daemon::perfile::evaluate(rules, memo, jobs, pool, progress) -> (Vec<Evaluated>, Shape)` replaces the per-pair `evaluate`; `Progress::{Served, Evaluated}`.
- `crossfile::{Worker, Snapshot}`; `CrossFile: Send`; `Engine::{cross_pending, has_pending_cross}` are gone; `Engine::{set_membership, edit, sync_git, sync_files}` take the partial-frame sink; `Engine::attribute`; `daemon::memory::{heaps, allocators, Purger}`; `memo::Memo::into_store`.
- facts-a depends on `rayon` (=1.12.0, the version rerail-engine builds).
- Harness: `FactsDaemon.request(header, body, partial)`, `edit(…, partial)`, `close(attribute)`; `run --engine facts` writes `summary` (first and complete, by bucket), `heldMismatches`, `memory`; `compare-findings` maps rule ids as the engine does, lists failures and sums differences by check.

### Commits

- p3c: streaming per-file findings, one evaluation queue over a persistent pool, cross-file thread, memory attribution and purge (this continuation).

## Resume notes

- Runs (`~/rerail-bench/facts/phase-3/p3c/runs/`): headline `20261023T1620-facts-first40-final16` and `…-final4`; `20261023T1545-facts-first40-pool16` / `…1555-…-pool4` (persistent pool, purge on the engine thread); `20261023T1605-facts-first40-stream` (streaming, threads per request, lower load); `20261023T1530-facts-first40-threads4` (threads per request at 4: the regex-cache evidence); `20261023T1550-facts-01-threads1` (programs alone); cross-file worker `20261023T1610-facts-cross-scratch-worker`; native parity `20261023T1510-native-first40-scan` (`run.sh`, `findings/NN.scan.json`, `compare-pool16.txt`); large-file attribution `20261023T1505-attr-sqlconn` (`out/`, `scale2/`). The run directories' time stamps are labels, not start times.
- Native addon: `target-addon/addon/rerail_engine.darwin-arm64.node` (`CARGO_TARGET_DIR=$PWD/target-addon npx napi build --platform --profile loop --manifest-path native/Cargo.toml --output-dir target-addon/addon`, untracked). Native per workspace: about 7 min and 5.6–7.8 GiB peak at this load.
- Open: the edit gate is the rule (§ 1 c: Jeremy for the shape; P3b for the class and the uncounted subtree searches); first findings of a 718 KB edit are bounded by its facts build (an incremental parse would be the lever); per-clone regex pools (§ 3); native's edit path with eviction (§ 5) is a native-store bug; P3a's model plugs into `crossfile::Worker` unchanged; the Spike B 1,111-event replay through the daemon is still not wired.
