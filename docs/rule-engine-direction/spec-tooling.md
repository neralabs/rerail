# Built-in tooling: memory limits, debugging, testing, profiling

Status: proposed part of `spec.md` (new §18). It follows `findings-contract.md` (outcomes `Complete | Unknown(reason)`) and the user's 2026-10-25 positions:
- there is no Metal → CPU/JIT fallback;
- resource exhaustion is reported and counts against coverage;
- memory limits, debugging, testing tools and profiling are first-class design parts.

The paths below are relative to the merged worktree (`Pulling-it-all-together`) unless prefixed:
- `N1562:` means `jbarnes/ner-1562-sql-rerailer@8b6576f0`;
- `SSC:` means `jeremy/sql-capture-demand-spikes@b87eba3b`;
- `Tempo:` means `nera-landing-2/ner-1562-scale-test-the-re-railer@e95613625`, under `tempo/`;
- `DLRE:` means the DLRE worktree.

---

## Part 1. Inventory: what exists

### 1.1 Spans and profiling (main + DLRE, merged)

| Piece | Where | What it does | Verdict |
|---|---|---|---|
| Cargo feature `code-rule-profile` | `native/Cargo.toml:11` | Compiles the native spans in. Off by default | **Reuse.** Keep as a feature for the allocation-heavy detail; always-on counters move to §2.6 |
| `profile::trace` spans | `native/src/code_rule_dsl/profile/trace.rs:9-136` | `RERAIL_PROFILE` env enables them. `Context::enter(rule, clause, path)` sets the attribution. `Span::start(phase, name)` emits wall and thread CPU. Output is the `RERAIL_TRACE <json>` line format on stderr | **Reuse the mechanism.** Replace the free-form phase names with the fixed stage taxonomy in §2.6 |
| Thread CPU timing | `profile/sql/timing.rs:1-20` (`Timing {wall, cpu}`, `CLOCK_THREAD_CPUTIME_ID`) | Sums worker CPU correctly under load | **Reuse unchanged** |
| Memory samples | `profile/memory.rs:1-30` | `getrusage` peak RSS plus jemalloc allocated and in-use, at fixed phase names (`producer-prepare`, `sql-input`, `sql-materialize`, …) | **Reuse.** Re-key to the new stages |
| Aggregate report | `profile.rs:578` (`RERAIL_CODE_RULE_PROFILE`) | A per-rule CPU report at the end of a run | **Reuse,** emitted as part of the unified profile (§2.6.4) |
| SQL statement profiler | `profile/sql.rs:1-330` (`RERAIL_SQL_PROFILE_DIR`) | Records every executed statement with SQLite counters. `explain_pending` runs `EXPLAIN QUERY PLAN` and `EXPLAIN` bytecode later, so the measurement isn't distorted | **Reuse unchanged.** It is the SQL-layer profiler |
| SQL coverage | `profile/sql/coverage.rs` | Statement coverage | Reuse |
| Navigation and semantic counters | `profile/navigation.rs`, `profile/semantic.rs` | Counters for matcher navigation and semantic lookups | Keep for the native matcher, which still serves rehydration and the oracle |
| `estimated_bytes` | `tape.rs:337,485,1417`, `lines.rs:128`, `relational/syntax.rs:188` | Per-object size estimates for caches | **Reuse the pattern.** Every cached or arena object reports bytes (§2.1) |
| jemalloc | `native/src/lib.rs:25-74` (global allocator, tree-sitter allocations routed through it); `tikv-jemalloc-ctl` stats | Allocator residency | Reuse |
| Budgets | `model.rs:36-49` (`max_visited_nodes`, per-file allowance); `eval/evidence.rs:248` | Work allowances | **Change meaning.** A budget now only produces `Unknown(budget)` (decision 0001). It is never semantics |
| Profiling CLI and docs | `docs/code-rule-profiling.md`; `npm run profile:code-rule-dsl` (`scripts/profiling/run.py`, `sample.py`, `report.py`, `compare.py`) | Pinned-snapshot runs, macOS `sample` attachment, bottom-up reports, comparison | **Reuse.** `run.py` becomes the driver for the unified profile format |
| SQL profile tooling | `benchmark/sql-profile/{inventory,coverage,execution-budget,execution-reduction,report,summarize,summary}.mjs`; `scripts/profiling/sql-{report,budget,stats,parity}.*` | Reports and parity over captured SQL | Keep `sql-parity.ts` and `report`. Delete the budget and reduction scripts tied to the authored-SQL executor when decision 0002 lands |
| Full-repository gate | `DLRE:scripts/profiling/workloads/full_repository.py` (committed in DLRE at `ab4d83b3`) | Cold full data-layer workloads. Per invocation it records `files, bytes, expected, actual, peakRssBytes, compilationMs, coldMs, phaseCpuMs` plus `/usr/bin/time -l` resource usage, into `manifest.json` and `invocation.json` | **Reuse as the measurement gate.** It consumes the unified profile (§2.6.4). Compile time is already separate |

### 1.2 Transducer and Metal (N1562, Tempo)

| Piece | Where | What it does | Verdict |
|---|---|---|---|
| Program validation | `Tempo:native/src/policy_dsl/transducer.rs:1534` (`validate`) | Table bounds, word-count limit (≤ 65,536), predicate and label consistency | **Reuse as the core of the verifier** and extend it (§2.3.2) |
| Launch guard | `Tempo:transducer/metal_launch.rs:18-185` (`DispatchRecord`, `LaunchScope`, `peak_application_buffer_bytes`); tests at `:296,335,376` show no encoder can bypass it, and a GPU-side counting kernel counts every dispatch API | Counts every dispatch, tracks the peak application buffer bytes, and refuses submission past the cap | **Reuse the accounting and the bypass tests.** Replace the fixed caps (200/600 launches, `METAL_BUFFER_LIMIT = 2_000_000_000` at `metal_backend.rs:22`) with plan-time bounds (§2.1) |
| Per-task status | `N1562:packed/metal_batch_runtime.metal:30-34` (`Status{error, resultCount, captureCount, programCounter, …}`, `Evaluation{outcome, failureArena, failureTask, failurePc, workUsed, quotas[14]}`) | Per-task error code, failing pc, failing arena and task, and work used | **Reuse the shape:** a failing task identifies itself (pc, arena, task). That is the basis for repro bundles (§2.2.5) |
| Arena quotas | `N1562:packed/metal_batch_abi.rs:318-396` (`EvaluationQuotaRecord`, 14 arena kinds); `metal_batch_runtime.metal:39-40` (`reserve_shared`, `reserve_transient`) | Atomic logical and physical reservation. Returns 1 when the quota is exceeded and 2 when physical capacity is exhausted | **Reuse the reservation idiom** for the set-stage arenas. The 14 per-path arena kinds go away with path enumeration (`research/metal-recursion.md`) |
| Semantic conformance | `Tempo:transducer/metal_semantic_conformance.rs` (+ phase test modules) | Metal against CPU on fixtures | **Reuse** as the seed of the executor differential suite (§2.4.1) |
| Design and status docs | `Tempo:docs/metal-*.md`, `transducer-*.md`, `packed-result-bottleneck-profile.md` | Recorded dispatch counts, memory and failures | History only |
| Dumps and disassembler | none found in Tempo or N1562 | — | **New** (§2.2.1) |
| Metal counters and GPU frame capture | none found (`MTLCaptureManager`, counter sample buffers absent) | — | **New** (§2.2.4, §2.6.2) |

### 1.3 SQL-side and parity harnesses

| Piece | Where | Verdict |
|---|---|---|
| `EXPLAIN` capture | merged `profile/sql.rs:293-298`; `SSC:sql/profiling.rs:371`, `SSC:sql/fused/profile.rs:122` | Use one implementation, the merged one |
| SQLite statement counters | `profile/sql.rs` (callbacks read counters only) | Reuse. Add `sqlite3_stmt_status` VM steps, sorts and autoindex as named fields |
| `dbstat` | not used | **New:** index size accounting (§2.6.3) |
| Data-layer replay and parity | `conformance/data-layer/replay.ts:176-186`, `suite.ts`, `incremental.ts` | Reuse. Convert to finding-key sets (`findings-contract.md` §5) |
| Language-pack scoring | `score.ts:39-58` | Reuse, as sets |
| Two-binding swap precedent | ast-index-spike native↔pure-SQL child function, 0 mismatches over 4,264 queries (`README.md`, Architecture 2); N1562 plain-table substitution for `rerail_matches` | The pattern for the binding differential (§2.4.1) |
| Fuzzing and property tests | none in `native/Cargo.toml` (no proptest, quickcheck or cargo-fuzz) | **New** (§2.4.3) |

---

## Part 2. Specification

### 2.1 Memory limits

**Model.** Every allocation the engine makes for evaluation belongs to exactly one budgeted pool. Each pool's limit is computed at plan time from statistics, then enforced at run time. Exceeding a limit never reroutes work. It yields `Unknown(resource: <pool>)` for the affected (program, document) pairs, which is reported and counted against coverage.

| Pool | Scope | Sized from | Enforced by |
|---|---|---|---|
| `tape.resident`, `source.resident` | process, shared host/device (no-copy slabs, counted once) | Σ node_count × v9np widths and Σ `raw_length` of the `snapshot` and `edit` residency classes (`spec.md` §20.7) | the planner runs the audit in residency waves; never `Unknown` for non-residency |
| `tape.derived` | process | `par`, `cidx`, line starts, text hashes per resident tape | sidecars evicted before their tapes |
| `batch.doctable` | per batch | batch document records | the planner splits the batch |
| `set.<stage>` | per batch, device | the set bound from kind statistics, propagated through operators (posting 4 B/element or bitmap n/8, whichever is smaller) | `reserve_*` atomic idiom (N1562), one quota per live set |
| `output.<relation>` | per batch, device | estimated rows (`xBestIndex` estimate) × record width, × a safety factor recorded in the plan | count → scan → scatter. Counts are exact before allocation, so overflow is impossible after the count pass. A count above the limit gives `Unknown(resource)` for the overflowing documents |
| `queue.<program>` | per batch, device | unbounded programs only (device-side enqueue). The bound is ≤ nodes × reachable states, recorded by the verifier | a GPU-side reservation that fails → the task records `Unknown(resource)` and stops enqueuing |
| `facts.<family>` | per document digest, host | `estimated_bytes()` of the fact relation | facts memo with LRU eviction under a global cap |
| `sql.temp` | per statement, host | SQLite `temp_store` and `cache_size` pragmas, per connection | SQLite `SQLITE_FULL` → `Unknown(resource: sql)` for that batch |
| `plan.cache`, `consolidation` | process, host | configured | LRU |

**Global caps.** `engine.limits.device_bytes` (default: 50% of `recommendedMaxWorkingSetSize`) and `engine.limits.host_bytes` (default: 25% of physical memory) are configuration, not constants in code. The planner packs batches so that Σ pool limits ≤ cap. The plan records each pool's limit, and the profile records each pool's high-water mark (§2.6).

**Rules:**
1. No pool may be sized by a hard-coded constant. Tempo's 2 GB `METAL_BUFFER_LIMIT` and its 200/600 launch caps are removed. The launch accounting and its bypass tests (`metal_launch.rs:296-376`) stay, now checked against the plan's reported dispatch count.
2. Every cache object implements `estimated_bytes()` (the existing pattern).
3. `Unknown(resource)` carries the pool name, the limit and the requested size. The coverage metric reports the share of `Unknown(resource)` per pool. CI fails if it exceeds the per-corpus threshold.

**CI gates** (phase gate only, per the test-budget rule):
- peak RSS and peak device bytes per gate workload, from `full_repository.py`, with thresholds recorded in the gate manifest;
- `Unknown` rate per reason;
- the reported dispatch count must equal the dispatch count observed by the launch guard.

### 2.2 Debugging

**2.2.1 IR dump and disassembly (new).** One stable text format, `rir` (Rerail IR), used by every tool. It is UTF-8, line-oriented and sorted, with no addresses or hashes except content digests, so it diffs cleanly.

```
program fn_has_yield  digest=sha256:…  ir=4  grammar=python@<digest>  caps=metal,jit,interp
  tables strings=[…] labels=[L0="match"] predicates=[…]
  0000  KIND        function_definition
  0001  EXISTS      @sub0
  0002  EMIT        L0
sub @sub0
  0000  DESCEND_KIND yield  stop=@stop_scope
  0001  ACCEPT
group g7  scope=sha256:…  members=[ruleA#c1, ruleB#c3]  dispatch={function_definition:0000, call:0040}
  labels L0→ruleA#c1  L1→ruleB#c3
  trie …
setplan batch-local
  stratum 0: S0 = postings(kind=yield)
  stratum 1: S1 = subtree_has(S0, stop=anc(function_definition|lambda|class_definition))
```

- `rerail engine dump --rule <file> [--clause <id>] [--fused] [--setplan]` prints the compiled programs, fused groups, label tables and set plans for a rule set.
- Golden tests compare dumps (§2.4.4).

**2.2.2 Assembler (new).** `rir` parses back into a program. Round trip is exact: `assemble(disassemble(p)) == p`. The assembler runs the verifier, so hand-written programs (repros, IR tests, executor bring-up) cannot bypass it.

**2.2.3 Plan explain (new; reuses the stored decisions).** `rerail engine explain --rule <file> [--path <file>]` prints every plan-time decision with the statistic values it read and their validity intervals. That is the `decisions` blob in `compiled_plans` (`spec-indexes.md` §2.12):
- anchor choice and the alternatives' counts;
- seeded vs scan;
- fusion groups and the cost-model terms;
- mask vs per-root scan;
- batch formation;
- pool limits.

With `--path` it also shows the document's scope result (Absent / Present / Unknown and why), its demand row, and its outcome. For the SQL layer it shows `EXPLAIN QUERY PLAN` for the composed statements (the existing `profile/sql.rs` path).

**2.2.4 Executors.**
- **CPU interpreter (the reference):** `--trace` emits one line per executed instruction (pc, node, kind, cursor, set-stage reads) for one (program, document). `--step` is an interactive single-step at the CLI over the same trace. Both are available only on the interpreter.
- **JIT:** `--dump-jit` writes the generated code next to its `rir` source, keyed by program digest.
- **Metal debug mode** (`engine.metal.debug = true`):
  - per-task status records stay enabled (N1562's `Status`/`Evaluation` shape: error, pc, arena, task, work used);
  - the reported and observed dispatch counts are compared;
  - each set-stage and queue arena's high-water mark is read back;
  - `MTLCaptureManager` capture of one batch can be requested with `--gpu-capture <batch>` (new). The capture file goes to the profile directory.

**2.2.5 Repro bundles (new).** Any of the following writes a self-contained bundle: an `Unknown(resource)`, a verifier rejection, an executor error, or a differential mismatch (§2.4.1). The bundle contains:
- the `rir` of the programs involved;
- the tape columns of the documents involved, plus source;
- the demand rows;
- the statistics snapshot used by the plan;
- the plan decisions;
- the expected and actual key sets.

`rerail engine replay <bundle> [--executor interp|jit|metal]` re-runs it with no store and no repository. The bundle is the unit attached to bug reports and turned into regression tests.

**2.2.6 Tape and index inspection (new).**
- `rerail engine tape --path <file> [--doc <ordinal>]` prints the v9np columns, the document table, source maps and sidecars, with node kinds named.
- `rerail engine index --check [--sample N]` recounts kind and term vocabulary and edge frequencies from tapes and compares them with `search_kind_vocab`, `search_term_vocab` and `edge_freq` (`spec-indexes.md` Q8).

### 2.3 Verifier and capability checks

**2.3.1** The verifier runs:
- at compile time, before caching;
- in the assembler;
- under `debug_assertions` at load time.

**2.3.2 Obligations:**
1. Table bounds and encoding limits (Tempo `validate`).
2. Every cycle makes progress.
3. Bounded `CALL` depth (E6).
4. The rule graph is acyclic, apart from monotone single-direction recursive groups.
5. Set mode only.
6. Every loop is bounded by a batch constant. Otherwise the program is classified **unbounded** and must declare a queue pool bound (device-side enqueue).
7. Each operator's capability flags. A program's capability is their intersection, and **every program must be Metal-capable.** A program that is not is a compiler bug, reported as a verifier failure; it is not routed to the CPU.
8. The pool sizes the plan declares cover the operators' static needs.

### 2.4 Testing

**2.4.1 Differential testing on the finding-key set.**
- **Comparison relation:** `findings-contract.md` §5. Key sets are compared on (rule, file) pairs that both sides report `Complete`; `Unknown` is counted separately.
- **Pairs:**

  | Pair | Purpose |
  |---|---|
  | interpreter ↔ JIT ↔ Metal | executor equivalence |
  | fused ↔ unfused | fusion legality |
  | vtab binding ↔ pure-SQL binding | two-binding rule (ast-index-spike swap precedent) |
  | engine ↔ native ast-grep matcher | semantics oracle |
  | engine ↔ data-layer oracle findings | port parity, via `conformance/data-layer/replay.ts` as sets |

- Any mismatch writes a repro bundle.
- **Corpora:** the existing fixture suites for the inner loop; the gate corpora (data-layer 9,021 files, CPython with 335 patterns) for phase gates.

**2.4.2 Golden dumps:**
- `rir` dumps for a fixed set of rules (one per frontend feature, plus the gate rules' heaviest clauses);
- plan-explain output against a frozen statistics snapshot;
- updated deliberately with a flag, never automatically.

**2.4.3 Property and fuzz tests (new):**
- **IR:** generate random verified programs, then check interpreter = JIT = Metal on random small tapes;
- **assembler:** round trip;
- **verifier:** mutate valid programs; anything that is accepted must run without out-of-bounds reads, and anything that is rejected must name a violated obligation;
- **tape:** random trees, then encode → decode identity (v9np), and v7 ↔ v9np node-for-node equivalence;
- **set stage:** random rule trees, then set-stage results = per-root walk results;
- **tools:** `proptest` for properties, and `cargo-fuzz` targets for the assembler, the tape decoder and the verifier.

**2.4.4 Tape and Blade:**
- v7 ↔ v9np round trip over the gate corpora (the v8 spike's checksum method, `research/tape-v8-measurements.md`);
- the Blade compiled-view tape compared node-for-node with tree-sitter-php's tape of the same text (decision 0003).

**2.4.5 Index consistency:** `rerail engine index --check` (§2.2.6) runs at phase gates and after migrations.

**2.4.6 Layout for targeted runs** (the test-budget rule). Tests sit next to the module they test, so the inner loop runs only the touched module:

| Module | Inner-loop command |
|---|---|
| `tape/` | `cargo test --lib code_rule_dsl::tape` |
| `engine/transducer/ir` | `cargo test --lib engine::transducer::ir` (dump/assembler/verifier goldens and properties) |
| `engine/transducer/exec/<backend>` | `cargo test --lib engine::transducer::exec::<backend>` (differential against the interpreter on fixtures) |
| `engine/plan`, `engine/stats` | `cargo test --lib engine::plan` / `engine::stats` (explain goldens against frozen statistics) |
| `engine/vtab`, `engine/compose` | `cargo test --lib engine::vtab` / `engine::compose` (binding differential on fixtures) |
| `engine/findings` | `vitest run test/engine/findings` plus `cargo test --lib engine::findings` |
| `content_store/` indexes | `cargo test --lib content_store::search_index` |

Full suites, gate corpora, fuzzing campaigns, Metal on the gate corpora, and `full_repository.py` run at phase gates only, once, in the background.

### 2.5 Logging of outcomes

Every (program, document) outcome is a row in `t_outcome`: `complete`, or `reason ∈ {budget, resource(pool), parse, unsupported, timeout, executor_error}`. Consolidation aggregates these into the coverage report:

```
coverage{rule, reason} = count of (rule, file) by outcome
```

The coverage report is part of the profile (§2.6.4) and part of the gate.

### 2.6 Profiling

**2.6.1 Stage taxonomy.** These are the fixed span names. Every span carries rule, clause, label and document attribution where it applies. The stages match the performance model:

| Stage | Covers |
|---|---|
| `compile` | rule compile, plan, fusion, and verification. Recorded separately |
| `plan.validate` | statistic point lookups for plan re-validation |
| `scope` | index lookups and candidate documents |
| `demand` | building demand rows |
| `upload` | tape columns and postings sent to the device |
| `program` | instruction programs, per group |
| `setstage` | each stratum |
| `queue` | device-side enqueue waves |
| `facts` | fact families: compute or memo hit |
| `sql` | composed statements, per statement |
| `consolidate` | findings consolidation |
| `rehydrate` | native matcher for payloads |

**2.6.2 Executor counters** (always on; cheap):
- **Interpreter and JIT:** instructions executed, nodes touched, paths pushed, per program and label.
- **Metal:** dispatches (reported vs observed), command buffers, waves per queue, GPU time per dispatch (`GPUStartTime`/`GPUEndTime` per command buffer, plus counter sample buffers where supported), arena high-water per pool, occupancy (threads per threadgroup × threadgroups vs device maximum), bytes uploaded and read back.
- **Fused groups:** per-label attribution of outputs and work, from the label bitset counts. Group cost is split over member rules in proportion to work counters, recorded as an estimate.

**2.6.3 SQL and storage.**
- Per statement: the existing `profile/sql.rs` counters plus `sqlite3_stmt_status` VM steps, full-scan steps, sorts and autoindex.
- `EXPLAIN QUERY PLAN` on first sight.
- Index and table bytes from `dbstat` when compiled in, otherwise page counts.

**2.6.4 One machine-readable profile.** JSON lines in one file per run, with record types:
- `run` (corpus digest, rule-set digest, engine version, machine);
- `span`;
- `counter`;
- `memory` (pool limit and high-water);
- `outcome-summary`;
- `stmt`;
- `decision` (plan explain records);
- `gate` (cold CPU, wall, peak RSS, peak device bytes, compile ms, parity).

`full_repository.py` writes and reads this format. `RERAIL_TRACE` lines become `span` records. The existing `RERAIL_CODE_RULE_PROFILE` aggregate becomes a summary over `span` records. `report.py` and `compare.py` consume it.

**2.6.5 Proxy metrics.** Statement counts, SQL bytes, VM steps, instruction counts and dispatch counts are recorded with `"diagnostic": true`. The gate reads only end-to-end CPU, wall, peak RSS, peak device bytes, parity, the Unknown rate, and compile time (reported separately). README, Measurement gate.

### 2.7 CLI summary

| Command | Purpose |
|---|---|
| `rerail engine dump` | `rir` programs, groups, label tables, set plans |
| `rerail engine asm` | assemble and verify `rir` |
| `rerail engine explain` | plan decisions with statistics; per-document scope and outcome; `EXPLAIN QUERY PLAN` |
| `rerail engine run --profile <dir> [--executor …] [--trace] [--step] [--gpu-capture n]` | evaluate with the unified profile |
| `rerail engine replay <bundle>` | re-run a repro bundle |
| `rerail engine tape` | inspect tape columns, documents and sidecars |
| `rerail engine index --check` | index consistency |
| `rerail engine diff --a <executor/binding> --b <…>` | differential run on a corpus, writing bundles on mismatch |

---

## Part 3. Reuse vs new

**Reuse:**
- main/DLRE: `profile::trace` span mechanism, `Timing`, memory samples, `profile/sql.rs` statement profiler and deferred EXPLAIN, the `estimated_bytes` pattern, jemalloc stats, `scripts/profiling/run.py`/`report.py`/`compare.py`, the data-layer replay harness;
- DLRE: `full_repository.py` as the gate;
- Tempo: `validate` as the verifier core, launch accounting and its bypass tests, the semantic conformance suite;
- N1562: the per-task status and evaluation records, and the `reserve_*` quota idiom.

**New:**
- the `rir` dump/assembler format;
- plan explain over stored decisions;
- interpreter trace and step;
- Metal counters and GPU capture;
- repro bundles and replay;
- plan-time pool sizing replacing the fixed caps;
- `Unknown(resource)` coverage accounting;
- the property and fuzz suites;
- the index consistency checker;
- the unified profile format;
- the `rerail engine` CLI.

**Removed:**
- Tempo's fixed 2 GB, 200- and 600-launch caps;
- N1562's 14 per-path arena kinds;
- the SSC duplicate EXPLAIN helpers;
- the authored-SQL budget and reduction scripts, after decision 0002.
