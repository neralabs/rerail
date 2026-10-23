# Phase 3, P3c: store and daemon

Branch `Moti-Velz/facts-engine-p3c-daemon` (nested worktree `.worktrees/p3c-daemon`), from integration `1c5a84b7a`. Run directories: `~/rerail-bench/facts/phase-3/p3c/runs/`. Counts first; every wall-clock below is uncalibrated (loaded machine, 18 cores, other slices running).

## Gate rows

| Id | Target | Measured | Result | Run dir |
|---|---|---|---|---|
| G3c.1 unchanged re-apply | 0 work | 40 re-syncs of the same commit: 0 changed paths, 0 clauses evaluated; 224,925 per-file re-edits by Git blob id (every tracked `.py`/`.rs` of the 40 worktrees): 0 clauses evaluated, 0 memo lookups, 0 `need_bytes`, 0 changed paths; p50 127 ms per worktree for all 5,600 requests | **pass** | `20261023T142823-facts-first40` |
| G3c.1 new worktree | ≤ 1 s | 39 worktrees after the first, synced from the Git ODB over the warm store: p50 2.2 s, p95 8.7 s, max 9.0 s; the cost is evaluating each worktree's new contents (p50 11 pairs not seen before, 2–74), not membership. A worktree whose contents are known (clone, or 30: 2 new pairs) syncs in 0.27–0.5 s | **fail** (the work is evaluation of new contents; see § Open) | same |
| G3c.1 per-file findings after an edit | p95 ≤ 100 ms | 800 harness edits (20 per worktree; comment appended to the minimum, median, p95 and maximum-size Python file in turn, hash first): all p50 2.0 ms, p95 2,469 ms. By bucket p95: minimum 3.5, median 12.5, p95-size 27.1, maximum 3,308 ms. Edits that evaluated (180; the others' content was known from another worktree): p50 12.3 ms, p95 3,405 ms; without the maximum-size bucket p95 48.1 ms | **fail** at the harness's mix (the 718 KB file costs seconds); pass for every bucket but the maximum | same |
| G3c.2 resident, 40 worktrees, steady state | < 1 GiB | **896 MiB** RSS after 40 syncs, 800 edits, rebalance (live heap 629 MB) **without the cross-file model**; with it: floor only (P3a's model not merged; the from-scratch stand-in holds findings sets only, transient while it runs) | **pass as a floor** (without P3a's model) | same; `20261023T143525-facts-cross-scratch2` (from-scratch, 2 worktrees) |
| G3c.3 per-file findings equal native's | 0 differences, every worktree | native 40-worktree run launched (60 rules, shared store, same edits); not finished at hand-off. `just rerail-workspaces compare-findings <native>/findings <facts>/findings <facts>/facts.json` compares per workspace, scan and after-edits | **open** (native run in progress) | native: `20261023T143432-native-first40`; facts: `20261023T142823-facts-first40/findings` |
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
3. **G3c.3**: finish the native run and `compare-findings`; native takes > 15 min for one worktree with the 60 rules on this machine (smoke `20261023T142524-native-smoke01` did not finish in that time), so the 40-worktree native run (shared store, so later worktrees reuse per-file results) needs hours.
4. **Cross-file**: P3a's model (facts-b `gen/hybrid.rs`) plugs in behind `CrossFile` once facts-b exposes a lib target (P3a's stated preference); its `apply(worktree, [(path, old hash?, new hash?)])` maps 1:1 onto `PathChange`. `FromScratch` costs one repository phase per push (≈ 54 s for worktree 01 single-threaded), so it is a reference, not a production path.

## Commits

- `497202f75` p3c: facts memo in the content store; content-pure clauses; facts-a repo on it.
- `37cc0b22f` p3c: facts daemon, protocol, Git ODB sync, membership, CrossFile, harness `--engine facts`, PROTOCOL.md.
- `6e85b3d61` p3c: compare-findings, pair/rule thread split.
- this report.

## Resume notes

- Runs: facts 40 `~/rerail-bench/facts/phase-3/p3c/runs/20261023T142823-facts-first40` (`facts.json`, `findings/NN.{scan,edits}.json`, `store.sqlite`); native 40 in progress `…/20261023T143432-native-first40`; from-scratch cross-file smoke `…/20261023T143525-facts-cross-scratch2`; repo baseline of worktree 01 with all 60 checks `…/20261023T141318-repo01-all60` (per-file costs: p50 4.1 ms, p95 58.5 ms, max 4.1 s single-threaded).
- Worktrees: `~/rerail-bench/memory47/workspaces/01…40` (+47) are git worktrees of `~/repos/nera-data-layer` (`setup --first 40`); `~/tempo-qe/workspaces47` are plain trees whose git links are broken.
- Native addon for the harness: `target-addon/addon/rerail_engine.darwin-arm64.node` (built from this tree's `native/`, untracked).
- Next: G3c.3 compare; heap attribution (Open 2); re-measure G3c.1 with the thread split; wire P3a's model; the Spike B 1,111-event replay through the daemon (events in `~/rerail-bench/facts/spike-b/edits/seed-5eedb/`, blake3 hashes, contents in `spike-b/contents/`) is not wired yet.
