# SSC fragment work (b87eba3b → c17d2e47): what it proves for the unified engine

## Summary

The SSC work gives strong evidence for the shape of the vtab boundary (§9). Native matching runs behind one batched virtual-table call per fragment per batch, and SQL sees only completed, typed outcomes. On the full workload that costs about native CPU, with exact finding parity. SQL plus boundary overhead is 4.9% of matching CPU.

It gives no evidence about speed, because the executor behind the boundary is the same AstMatcher as the native path. The result is 1.02× native, not a gain.

It contradicts the spec in one place: 17 of 146 clauses run "independent structural predicates" as SQL over a per-node `node` table. The code to merge is `sql/fragments/`, `sql/engine/` and the workspace budget and partitioning, mostly into `engine/vtab/`, `engine/plan/`, `engine/stats/` and `engine/fallback/`.

The worktree is clean: no uncommitted changes and no stash for this branch. HEAD is c17d2e47. I modified nothing.

## 1. What the new work does, and what it measured

### e32b280b, "Retire SQL interpreters and spike batched matcher fragments"
This is the pivot that `research/sql-vs-transducer.md` saw uncommitted; it is now committed.
- **Removed** the generic finite, mixed-recursive, parse-recursive and pure-instruction-loop SQL emitters, plus the utility-projection adapter: about 3.7k lines across `sql/relational/outcomes/*_emit.rs`, `finite_*`.
- **`outcomes/retired_control.rs`** keeps the old control IR but every emission entry returns an error ("SQL state machine removed: use a batched matcher fragment contract").
- **Decision** (`docs/sql-matcher-fragment-transition.md:10-13`): "The generic SQL state machine is a rejected compiler architecture. SQL must compose typed relations… Coherent matching operations remain inside batched native matcher fragments."
- **Spike result:** all 11 previously refused grammars pass on 597 fixture files, 196,678 roots, 226 matches. Each grammar's table is called once, and no matcher instruction crosses back into SQL.
  - Per-grammar median CPU sums to 232.1 ms native vs 240.5 ms through SQL (`doc:118`).
  - Combined job (`doc:144`; `benchmark/sql-matcher-fragment-spike.json` `wholeJob.medians`): CPU 288.2 ms native vs 287.5 ms hybrid. Breakdown: setup 84.7 ms, prepare 0.175 ms, matching 200.6 ms. One statement, 1,625 SQL bytes, 11,873 VM instructions.

### c17d2e47, "Integrate typed matcher fragments with bounded SQL execution"
This replaces the public SQL compiler and session (`sql/mod.rs` now exports `engine::{Compiled, ProgramCompiler, Session}`).

- **`sql/fragments/` (1,208 lines)**
  - `Fragment` plus an eponymous vtab with schema `file,node,truth,ordinal,c0..cN,request HIDDEN,demand HIDDEN`. The capture columns are per fragment and declared only.
  - Three filter modes (`fragments/mod.rs:540-603`):
    - whole scope, built one file at a time;
    - `request IN (…)` through `sqlite3_vtab_in`;
    - `demand = handle`.
  - Requests are grouped by file and sorted by root. Each file runs inside `with_file_memo` and `begin_clause`/`end_clause`.
  - Outcomes are 1 / 0 / 2 (logical unknown), with negative tags for resource failures (`failure.rs`; −5 is deadline).
  - A file's provisional rows are discarded if that file fails (`mod.rs:316-338`).
  - `demand.rs` is an aggregate UDF that builds an immutable typed demand (node, list, count, span inputs) and returns a single-use handle. Limits: 1M requests or 128 MiB. An empty aggregate is an empty batch, never a scan.
  - `candidates.rs` is a root-domain vtab, used only for the structural-SQL arms.
- **`sql/engine/` (1,515 lines, of which `qualification.rs` and `tests.rs` are test-only, 539 lines)**
  - `planning.rs::Template::compile` splits each clause into:
    - a pure structural `Predicate` lowered to SQL (`structural.rs`: kinds, all/any/not, has/inside/precedes/follows with `neighbor`/`end` stop, unbound count);
    - a native remainder that stays an AstMatcher fragment.
  - `queries()` emits `UNION ALL` publication with 128 arms per statement (`ARMS`).
  - Compile cache keyed by `(source_digest, VERSION, each)`, LRU 1,024.
  - `Session` prepares once, replaces batch slots between runs, and re-prepares only when a statistics-driven decision flips (`engine.rs:333-345`).
  - `statistics.rs` plus `ContentStore::search_kind_counts` (`content_store/search_index.rs:+23`) does one `fts5vocab` point lookup per requested kind, through `json_each LEFT JOIN`.
- **Workspace changes**
  - `workspace/sql.rs`: hard budget of 100 matching SQL executions per apply; the producer pass reserves half for the consumer pass.
  - `workspace/evaluate.rs`: resident partitions sized at 4 MiB of source, so plans are reused across partitions. The number of statement executions is set by SQLite's compound limit, never by file count.
  - Per-file suspended budget is passed through, and `Unknown` maps to `budget-exceeded` failures.
- **Other files:**
  - `matcher::fragment_at` and `run::enumerate_bound` (rematch from the original inputs, never from the previous output);
  - `compile/mod.rs` `source_digest` (SHA-256 of the AST, language, gates and utility definitions);
  - `producers/mod.rs` copied from the integration tree, adding only `OptionValue::Text`.

### Full-workload result
Source: `benchmark/sql-matcher-fragment-integration.json`, cited as `int.json` below, and `doc:300-372`. Workload: 5,725 files, 95.7 MB, 146 grammars from 138 policies (the shipped Python catalog, not the data-layer rules). Build: opt-level 1, jemalloc. Single run.

| Measurement | Native | SQL fragments | Source |
|---|---:|---:|---|
| Initial-job CPU | 109.53 s | 111.79 s (1.021×) | `int.json native.cold`, `sql.cold`, `coldCpuRatio` |
| Wall | 107.49 s | 60.27 s | same. Not a fair comparison: SQL uses 4 worker partitions |
| Peak RSS | 684 MB | 1,233 MB (was 2,933 MB before partitioning) | `doc:310,317` |
| Median edit CPU, whole apply | 180.1 ms | 202.6 ms (1.125×) | `int.json edits[]` |
| Findings | 97,798 | 97,798, identical key hash `f1c746f8…` | `int.json` |
| Matching SQL executions | — | 46 (23 partitions × 2 statements) | `matchingStatements` |
| Compile and prepare | 129 ms compile | 151 ms compile; 6.32 ms prepare for 8 statements; 2.87 ms lowering and assembly | `sqlSessions`, `doc:367-368` |

Matching CPU breakdown (`int.json`): 83.56 s in total, of which 79.50 s is inside native boundaries and **4.06 s (4.9%) is SQL plus boundary overhead**. Other counters: 73.3M VM instructions, 3,289 xFilters (143 fragments × 23 partitions), 73.2M candidate roots, peak 133,408 request rows.

Numbers I derived from the evidence files:
- **Native AstMatcher costs 1,086 ns per candidate root** on the workspace (79.50 s / 73.2M) and 509 ns per root on the catalog fixtures (876 ms / 1.72M, from the catalog `.gz` boundaries).
- **Arm split** in the generated SQL (`benchmark/sql-matcher-fragment-integration.sql`):

| Arm type | Clauses |
|---|---:|
| Whole clause native | 129 |
| SQL structural prefilter feeding a native remainder by demand handle | 14 |
| Pure SQL structural | 3 |

- **Statistics were not used in the full run.** `compiler.statistics` is empty, so all 146 `descendantAnchors` are false. The `fts5vocab` path is exercised only by the test `fragment_integration_statistics_flip_shape_not_findings`.
- **Rules with no matches dominate.** The largest fragments include ASYNC221 (5.04 s, 4.56M roots, 0 rows), F633 (2.68 s, 0 rows), B007-uncertain (2.55 s, 0 rows), ASYNC230 (1.96 s, 0 rows) and ASYNC222 (1.93 s, 0 rows). The top 10 total 31.0 s, which is 39% of native boundary CPU (`largestNativeFragments`).

Other results:
- **Catalog:** 87,162 rule/file pairs and 2,477 raw matches, with 0 mismatches and 0 unknowns. CPU 1,034 ms native vs 1,116 ms SQL (catalog `.gz` `runs[0]`).
- **Validation:** SQL suite 176 passed, 0 failed, 23 ignored; 16 fragment checks, 11 hard cases, 10 three-valued checks, 4 registry checks (`int.json validation`).
- **Checker:** `benchmark/check-matcher-fragment-integration.py` enforces at most 100 executions, ≤ 1.30× cold CPU, ≤ 1.50× edit CPU and ≤ 1.5 GiB RSS. It also requires parity, complete partition traces, and source and binary hashes.

### Defects found along the way (`doc:239-296`)
- A logical unknown was being mapped to incomplete coverage.
- A global `ORDER BY` over the `UNION ALL` kept every producer cursor alive. Removing it did not fix the memory.
- The real memory problem: restored navigation data was 760,717,400 bytes across 4 partitions, because decoded tapes and semantic caches were retained for every file at once. Resident partitioning fixed it.

## 2. What it confirms, contradicts or refines in our model

### Confirms
- **§9 boundary and principle 1.**
  - One xFilter per (fragment, batch): 3,289 = 143 × 23.
  - Completed typed output only. Narrow per-fragment schemas, with a union schema only at final publication.
  - With this shape, SQL as the driver costs 4.9% of matching. The old state-machine emitters cost about 3× native (`research/sql-vs-transducer.md` §0).
  - This is the first full-repository proof that the boundary is cheap when native code does the matching.
- **§13 and dead end D73 (no generic interpreter in SQL).** The author removed the interpreters, and the 11 former refusals pass natively. Ledger rows 16 and 19 are settled on this branch.
- **§12 and decision 0001.**
  - The doc's "Integration work" section adopts set semantics and budget → `Unknown`, and drops exact work charges and witness order as requirements (`doc:183-186`).
  - One migrated test differs only in an attempted-witness metric after exhaustion, and is accepted on findings plus coverage (`doc:361-365`).
- **§11 plan cache.** Only a changed decision re-prepares; numeric drift alone does not (`engine.rs:333`). Compilation is cached by digest + compiler version, and the cache key does not imply sharing between clauses (ledger row 9).
- **§19 shape.** The executor is the whole cost: 79.5 of 83.6 s is native matching. The boundary is not where to look for the 10× gain; the AST row (M2) is.
- **§5.1.** The author states the `k%04x` tokens overlap across dialects, so counts are "cost estimates, not exact language frequencies" (`doc:225`). That is direct evidence for global kind codes.

### Contradicts
- **Structural predicates in SQL.** Spec §7 says "If a rule feature cannot lower, the IR is extended. It is never composed in SQL." Spec §9 forbids rows per node.
  - SSC lowers kinds, not, has/inside/precedes/follows and count to correlated `EXISTS` and `count(*)` subqueries over the per-node `node` vtab, for 17 clauses (`structural.rs:96-162`).
  - Their cost is not measured separately: it sits inside the 4.06 s overhead (136 ms of 1,012 ms on the catalog).
  - It works on this catalog, but it is the wrong boundary for the main engine. Its correct home is the pure-SQL fallback (§13), where it is a good, specialised, non-interpreter emitter. Each predicate maps directly onto IR opcodes: E2 `EXISTS`/`NOT_EXISTS`, E3 axes, E4 `COUNT`, E12 `DESCEND_KIND`.
- **Output contract.** SSC registers its fragments as `multiplicity: bag`, ordered by (file, node, ordinal), and sorts each file's matches by root (`planning.rs:96-120`, `engine.rs:501`). Spec E1 and §12 say set mode with no order. The ordinal is needed only for `each` enumeration.
- **Unit of batching.** It is one fragment per clause, with no program groups and no label demultiplexing (§11). Principle 5 (batch across rules within identical scope) and the fusion forms of §7 are not exercised.

### Refines
1. **Matcher/SQL boundary rule** (`doc:54-56`): "Keep capture rollback, recursive utility calls, and ordered searches inside a fragment when they share state." This is our "everything in one document lowers to the IR" applied at the AstMatcher level. It also gives a concrete test for whether a split is legal: a fragment is cut only where no rollback, utility or ordered-search state crosses the cut.
2. **"Typed matcher fragments" is the per-program shape of `t_match` plus `t_export_<schema>`.**
   - Typed capture cells (node, list, span, count) keep their document identity; a span from another file is rejected (`engine.rs:458`).
   - Output columns are only the declared captures.
   - The registry contract adds `completion: domain`: absent rows within the requested domain mean false. That is the right reading of set semantics plus a separate outcome relation.
3. **Capture-bearing demand (E7) needs a demand handle, not only `vtab_in`.**
   - `sqlite3_vtab_in` carries one scalar column. Tuples of (request, file, node, typed inputs) go through an aggregate UDF that returns a single-use handle, read by an equality constraint on a hidden `demand` column.
   - Properties: expiry; handles cleared on success and on failure; a second consumption is rejected; a transport budget.
   - This is the first working design for E7 and is worth adopting verbatim.
4. **Three outcome classes, not two.** Matched; logical unknown (a completed verdict from the three-valued matcher, never coverage loss); resource failure (`Unknown(reason)`, which discards that file's provisional rows).
   - Decision 0001 excludes three-valued semantics, but the current frontend still produces logical unknowns (`three_valued` grammars).
   - SSC found that treating them as incomplete coverage was a bug, and drops `truth=2` at publication (`WHERE truth=1 OR truth<0`).
5. **Batching is bounded by resident memory, not file count.**
   - The SQL path's RSS was dominated by decoded tapes and semantic caches (760 MB of navigation data across 4 partitions).
   - The fix: about 4 MiB of source per partition, with prepared plans reused across partitions. RSS went from 2,933 to 1,233 MB at unchanged CPU.
   - Statement executions are capped by an explicit job budget (100), and their number follows SQLite's compound-select limit (128 arms), never the file count.
   - The SQL path still uses 1.8× native RSS. That is unexplained and left open.
6. **Measured unit costs for §19:**
   - Native AstMatcher costs 0.5–1.1 µs per candidate root, at opt-level 1, with root-kind-only candidate selection and no file pruning.
   - Boundary plus publication costs about 55 ns per candidate root (4.06 s / 73.2M), or about 1.2 ms per xFilter.
   - Preparation is negligible: 6.32 ms for 8 statements.
7. **Fusion evidence on a second ruleset (my inference, not measured fusion).** On the Python catalog, rules that match nothing and anchor on very common kinds (4.56M roots, which looks like `call`) take 39% of native CPU in the top 10 fragments alone. That is the case for root-kind dispatch and tries plus shared sub-rules. §7 item 3 says tries "are not expected to pay" on the data-layer ruleset; for this catalog the evidence points the other way, so the claim should be scoped per ruleset.
8. **Per-edit measurement scope.** The 180–203 ms edit figure is a whole workspace apply, including the response with about 97.8k violations. It is not comparable to the fusion model's 2.9 ms p50, which is AST matching only. §14 and §15 must name what an edit measurement includes.
9. **Line-count correction for `research/sql-vs-transducer.md`.** It sized the then-uncommitted pivot at 614 lines plus 419 lines of tests. It is now `sql/fragments/` (1,208 lines) plus `sql/engine/` (1,515 lines). `sql/` still totals 104,176 lines, because the frame compiler is kept "for historical tests".

### Not proven by this work
- Any speedup: the executor is unchanged.
- Fusion or program groups.
- SQL composition across fragments at workload scale. Only the unit test `fragment_spike_composes_typed_outputs_with_join_antijoin_and_aggregate` covers it.
- Statistics planning in the full run (statistics were empty).
- Pure-SQL bindings for the sequential contracts (stated as unresolved, `doc:357`).
- The data-layer rules, and an optimised build.

## 3. What to merge, mapped to the §16 module map

| SSC source | Target | Notes |
|---|---|---|
| `sql/fragments/mod.rs` (vtab `Table`/`Cursor`, three filter modes, per-file grouping, failure isolation, `Metrics`) | `engine/vtab/transducer.rs` | The boundary skeleton. Swap the `matcher::fragment_at` call for transducer program-group execution. Keep the AstMatcher binding behind the same contract as the interim executor and parity oracle |
| `sql/fragments/demand.rs` (`DemandStore`, aggregate handle) | `engine/vtab/demand.rs` | Spec E7 / §9 capture-bearing demand |
| `capture_cell.rs`, `span_capture.rs` | `engine/vtab/` (typed `t_export` cell codec) | |
| `capture_env.rs` | `matcher/` (oracle-binding adapter) | Builds a `MetaVarEnv` from typed inputs; AstMatcher-specific |
| `failure.rs` | `engine/vtab/` → `t_outcome.reason` codes | |
| `candidates.rs`, `engine/structural.rs` | `engine/fallback/` | Pure-SQL binding of the structural subset. Also reuse its `Predicate` as a lowering checklist in `engine/transducer/lower/`. Not an execution path |
| `engine/planning.rs`: `Template` split, `decisions()` | `engine/plan/` | |
| `engine/planning.rs`: `queries()` publication | `engine/compose/` | |
| `engine/planning.rs`: `manifest()` | `engine/relations/` (DLRE `producers/`) | |
| `engine/statistics.rs`, `ContentStore::search_kind_counts` | `engine/stats/`, `content_store/` | Change to global `k%06x` tokens (§5.1). Add the decision-interval storage of `compiled_plans` |
| `engine.rs` (`Session`, prepare once, slot replacement, decision-gated re-prepare, scope-escape check, compile cache) | `engine/plan/` (in-memory precursor of the plan cache) plus a session in `engine/vtab/` | Drop the per-file `sort_by_key` (set semantics) |
| `workspace/sql.rs` execution budget; `workspace/evaluate.rs` resident partitioning and `Unknown` → failure mapping | integration workspace / `engine/plan/` batch formation; spec §18 memory pools | |
| `compile/mod.rs` `source_digest` | `engine/plan/` identity input | |
| `matcher::fragment_at`, `run::enumerate_bound` | `matcher/` | |
| `producers/mod.rs` | already in the integration tree | Take only `OptionValue::Text` |
| `benchmark/check-matcher-fragment-integration.py`, `workspace/fragment_integration.rs` (one subprocess per engine, provenance hashes, ceilings, add / add / restore edit control) | §15 / §18 measurement tooling, next to `full_repository.py` | |
| `sql/relational/fragment_tests.rs`, `sql/engine/tests.rs`, `qualification.rs` | differential and parity tests | |
| `retired_control.rs`, the frame compiler and the remaining state-machine IR | **do not merge** | Already marked delete in `architecture.md`. Keep the SSC history only |

Update `architecture.md` phase 4 to merge at c17d2e47, not b87eba3b. Its scope grows to include the boundary (`fragments/`, `engine/`). Today it lists only "emitter, statistics reader, cross-file".

## 4. Recommended spec text changes

1. **§9, add after "Demand in":**
   > Capture-bearing demand uses a **demand handle**. An aggregate function `demand_<group>(request, doc, root, inputs…)` builds an immutable typed request relation (node, list, count and span cells, each carrying its document identity) and returns a single-use handle. The vtab receives it through an equality constraint on a hidden `demand` column. Handles expire at batch end, are cleared on success and on failure, and reject a second consumption. An empty aggregate is an empty batch, never a scan. Transport is bounded (SSC: 1M requests / 128 MiB). Scalar demand uses `sqlite3_vtab_in`. (Evidence: SSC `sql/fragments/demand.rs`; test `fragment_spike_sql_capture_demand_and_bound_enumeration`.)
2. **§9 relations table:** add the `completion: domain` rule ("absent rows within the requested domain mean false; only `t_outcome` reports incompleteness"). Also: "a resource failure discards that (program, document)'s provisional rows. Other documents in the batch publish normally."
3. **§9, new paragraph "Measured cost":**
   > With completed output only, SQL plus boundary overhead was 4.06 s of 83.56 s matching (4.9%) over 73.2M candidate roots and 3,289 xFilters on 5,725 files; finding parity was exact (SSC `benchmark/sql-matcher-fragment-integration.json`). The SQL state-machine emitters this replaced ran at about 3× native.
4. **§6 E11, extend:**
   > Three outcome classes: matched, logical unknown, resource `Unknown(reason)`. A logical unknown is a completed verdict and never counts against coverage. Until frontends stop producing three-valued results (decision 0001), the boundary publishes logical unknown as no match and keeps it available only inside in-document composition.
   
   Record this as a decision; it is otherwise in tension with "three-valued logic as semantics" being excluded.
5. **§7 Frontends, add:**
   > Fragment boundaries follow shared state: capture rollback, recursive utility calls and ordered searches that share state stay in one program. Structural predicates (kinds, all/any/not, relations with `neighbor`/`end` stops, unbound counts) lower to E2/E3/E4/E12. **They are not composed in SQL over a node relation in the main engine.** SSC's structural-to-SQL lowering is the §13 fallback emitter for that subset.
6. **§7 measured list, item 3:** scope the claim: "not expected to pay on the data-layer ruleset". Add: "On the shipped Python catalog, fragments with no matches anchored on very common kinds take 39% of native time in the top 10 alone (SSC `largestNativeFragments`). Re-evaluate tries per ruleset from `plan_feedback`."
7. **§8 and §18 Memory:** add a **resident-batch pool**:
   > CPU batches are cut by resident bytes (decoded tape, navigation and semantic caches), not by file count. SSC measured 760 MB of navigation data across 4 whole-job partitions. A 4 MiB-source partition target cut peak RSS from 2,933 MB to 1,233 MB at unchanged CPU, with prepared plans reused across partitions.
8. **§11, add:**
   > A job has an explicit budget of matching statement executions, checked before execution and shared across producer and consumer passes. Statement count follows SQLite compound limits (SSC: 128 arms), never file or root count. A statistics refresh re-prepares only when a stored decision changes.
9. **§5.1, add evidence:** "SSC's `fts5vocab` reader had to treat counts as estimates because `k%04x` tokens overlap across dialects (`docs/sql-matcher-fragment-transition.md:225`)."
10. **§12:** state that the ordinal column exists only to key multiple witnesses of an `each` rule, that boundary outputs carry no order, and that consolidation, not the engine, sorts.
11. **§13, replace "SSC's emitter is the starting point":**
    > Starting points: SSC `sql/engine/structural.rs` and `fragments/candidates.rs` for the structural subset (specialised extent and parent joins), and the surviving non-recursive `sql/relational/` emitters. Sequential contracts (recursive utilities, capture rollback, ordered search, parsed regions) have **no pure-SQL binding**. SSC removed the interpreters that covered them, and the fallback reports them as uncovered with a coverage metric rather than emulating them.
12. **§15:** adopt the SSC checker pattern: explicit ceilings (cold CPU ratio, edit CPU ratio, peak RSS, execution count), source, binary and corpus hashes, one process per engine for RSS, and an edit control of add / add / restore with retraction checked. Also state that an edit measurement names its scope: matching only vs whole apply including output.
13. **§19.1 table, add measured rows:**
    - "Native AstMatcher (current baseline): 0.5–1.1 µs per candidate root (opt-level 1; SSC catalog and workspace)."
    - "vtab boundary plus publication SQL: about 55 ns per candidate root, about 1.2 ms per xFilter (M)."
    
    **§19.2 reconciliation:** cite this as a third data point. The current native executor is the same order as the fusion harness, which confirms that executor speed (M2), not the boundary, decides the AST row.
14. **§16 module map, Source column:**
    - `engine/vtab/transducer.rs` ← "SSC `sql/fragments/` (boundary, demand handle, typed cells) at c17d2e47";
    - `engine/plan/` ← add "SSC `sql/engine/` session and compile cache";
    - `engine/stats/` ← "SSC `engine/statistics.rs` + `search_kind_counts`";
    - `engine/fallback/` ← "SSC `engine/structural.rs`, `candidates.rs`".

    Also correct `architecture.md` phase 4 and the SSC row of `research/sql-vs-transducer.md` §2.1 to the committed paths and line counts.