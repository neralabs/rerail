# Phase 1: de-risk with two spikes

Objective: measure, before any compiler is written, the two claims the architecture rests on.

- **A.** Extracting facts once per content and evaluating the heaviest per-file rules as in-memory joins over those facts is ≥ 10× cheaper than today's native path, with oracle parity.
- **B.** Cross-file rules maintained as base + per-worktree delta dataflow keep 40 worktrees exact, under 200 MB resident, at ≤ 50 ms per edit.

Both spikes are throwaway in code, permanent in numbers. Their reports and schemas seed Phase 2.

## Base, branches, places

| Item | Value |
|---|---|
| Base commit | `Moti-Velz/facts-plan` (from `Moti-Velz/sql-dl60` at `e6d2ac73c`, plus these docs) |
| Spike A | worktree `~/orca/workspaces/rerail/facts-spike-a`, branch `Moti-Velz/facts-spike-a`, crate `spikes/facts-a` |
| Spike B | worktree `~/orca/workspaces/rerail/facts-spike-b`, branch `Moti-Velz/facts-spike-b`, crate `spikes/facts-b` |
| Reports | `docs/facts/reports/spike-a.md`, `docs/facts/reports/spike-b.md` in the spike's own worktree, committed |
| Raw runs | `~/rerail-bench/facts/spike-a/runs/<timestamp>/`, `~/rerail-bench/facts/spike-b/runs/<timestamp>/` |
| Fact files (shared format, § Contract) | `~/rerail-bench/facts/store/<hash[0..2]>/<hash>.facts` |
| Corpus for A | `~/rerail-bench/nera-data-layer-2b1504ad` (9,010 files, the commit Jeremy's recordings use) |
| Corpus for B | `~/tempo-qe/workspaces47` (47 worktrees; use 40) |
| Native attribution for batch 6 | `~/rerail-bench/dl60-split/b6-stage-builds.csv`, `b6-extraction-per-input.csv`, `b6-tables.md`, `b6-analysis.json` |
| Jeremy's rules and harness | `packs/data-layer/candidates/rules/*.json`, `docs/data-layer-batch-six.md`, `fixtures/host/data-layer-port/`, `conformance/data-layer/{replay,suite}.ts` (`--engine native|sql`), `scripts/dsl/data_layer_suite_record.py` |
| His shared syntax facts | `native/src/code_rule_dsl/relational/syntax.rs` (`Facts::build`), `syntax/packed.rs` |

## Contract (both spikes; Phase 2 freezes v1 from this)

- **Content hash**: blake3-256 of the file bytes, lowercase hex.
- **Node ids**: `u32` preorder index within the file's tree-sitter parse, root = 0. A node's descendants are exactly the ids in `(id, id + subtree_size]`. Byte offsets half-open `[start, end)`. Lines 1-based.
- **Strings**: interned per process into `u32` symbols. Facts are columnar arrays of integers. Node text is recovered from the source slice on demand, never copied into facts.
- **Naming**: snake_case relations, columns documented as `name:type`. Per-file facts are implicitly keyed by content hash: one fact set per content.
- **Classification**: exported cross-file facts are the only cross-file inputs. A per-file rule never reads them.
- **Fact set v0 skeleton.** A owns the syntactic and Python-derived part; B owns the exported part. Each spike's report contains its final schema as a table. Coordinate names by message.
  - Syntactic: `node(id, kind:sym, parent:id?, start:u32, end:u32, start_line:u32, end_line:u32)`, `child(parent:id, index:u16, child:id)`, `field(parent:id, field:sym, child:id)`, `identifier(id, name:sym)`, `line(no:u32, start:u32, end:u32, indent:u16)`.
  - Python-derived (A confirms which batch 6 needs): `function`, `class`, `parameter`, `call`, `argument`, `attribute`, `assignment`, `decorator`, `import`, `string_lit`, ownership via id ranges.
  - Exported (B): `def(name:sym, kind:sym, qualname:sym, line:u32)`, `ref(name:sym)` distinct per file, `import(module:sym, name:sym?)`, `call_edge(caller:sym, callee:sym)`.
- **Fact file format**: one file per content hash, any compact binary, documented in the report, readable by the other spike's loader in Phase 2. Include the schema version in the header.

## Measurement protocol

- Release profile. `export CARGO_BUILD_JOBS=4`; `CARGO_TARGET_DIR` inside your worktree.
- Every timing run wrapped: `~/rerail-bench/bench-lock <command>`. Record `uptime` load before and after; label a run untrusted if the 1-minute load exceeded 8.
- Three runs per timing claim; report the median and the spread.
- Peak RSS: `/usr/bin/time -l` (macOS, `maximum resident set size`).
- No single run over 15 minutes; split by batch or worktree subset.
- Each run's command line, stdout, and a `results.json` go under the run directory. The report's results table cites the run directory for every number.
- Compare like with like: the native numbers above are release builds on the same corpus; say so when yours differ.

## Spike A: facts once + in-memory relational evaluation

### Scope

Batch 6's 10 rules (listed in `docs/data-layer-batch-six.md`; confirm against `fixtures/host/data-layer-port/batches.json`). All 10 are the target. Minimum acceptable: the rules covering ≥ 70% of batch-6 native CPU, ratio computed on that set, with the apportioning written down (per-rule global-phase time from the split CSVs plus a share of the 122.8 s file-phase matching apportioned by matcher-input count).

### Steps

1. **Enumerate and census.** For each batch-6 rule: its clauses, matcher inputs, stages. Classify every stage: (a) AST-schema relation that becomes an extracted fact, (b) rule logic derivable from facts, (c) bespoke, needs a fact not in v0. Table with counts. This is the first commit.
2. **Fact schema v0 and extractor.** Build on `Facts::build` where it fits (packed columns, kind postings, fields). Add the Python-derived relations the census needs. Per-file, streaming: a file's facts are built, used, and dropped; nothing is collected across files. Persist to the fact-file format for reuse by B and Phase 2. Measure extraction time and bytes per file.
3. **Evaluator.** Rules as Datalog over facts. Preferred: the `ascent` crate (compile-time, semi-naive, stratified negation, aggregates); hand-rolled Rust joins are acceptable where Ascent cannot express something. The choice and its implications for a runtime-loadable evaluator (Phase 2) go in the report. No SQLite, no per-statement planner. Keep each rule's cost measurable on its own.
4. **Port the rules** faithfully to their SQL semantics. Mechanical translation of stages to rule text may be handed to a `sonic` subagent with the census table and the schema as the spec; the executor verifies every rule by running it against the oracle.
5. **Parity.** Emit findings as JSON with the same keys `replay.ts` compares: check, code, path, line, multiplicity, plus column or category where the recording has them. Compare offline against `fixtures/host/data-layer-port/batch-06-recording.json` (1,259 calls, each over its own input files) and the repository oracle (173 findings at `2b1504adc`; re-record with `data_layer_suite_record.py` if it is missing from `/tmp/dl60/`). Write the comparator so it reuses `replay.ts`'s matching rules, not a looser one.
6. **Speed.** Repository run over all 9,010 files with 4 workers: parse + extract + evaluate, streaming per file. Per-rule and per-clause CPU, total CPU and wall, peak RSS.
7. **Report** with the template below.

### Gates

| Id | Gate | Target |
|---|---|---|
| A1 | Parity, per ported rule, source corpus and repository | 0 false positives, 0 false negatives, 0 failures |
| A2 | Repository CPU for the ported set, parse + extract + evaluate | ≤ 10% of native CPU for the same rules (≤ 40 s if all 10; 393 s baseline) |
| A3 | Per (clause, file) evaluation | mean ≤ 0.5 ms, p95 ≤ 5 ms |
| A4 | Fact extraction | p50 ≤ 10 ms per file, p99 ≤ 100 ms; bytes per file reported |
| A5 | Peak RSS, repository run, 4 workers | ≤ 2 GiB (28.6 GiB baseline) |

### Kill criteria

- A2 between 10% and 25% with A1 passing: the evaluator replaces only the SQLite stages; pattern matching stays native. Phase 2 scope shrinks accordingly.
- A1 unreachable for an AST-schema relation: that relation is wrong in v0; fix the schema, not the rule.
- A4 p50 > 30 ms: the schema is too rich for always-on extraction; split into core facts plus on-demand derived facts.

### Also report

- The stage census (step 1) with per-category counts: it sizes Phase 2's compiler.
- Which SQL constructs the ported rules needed (joins, antijoins, group counts, recursion, string ops, window functions).
- The per-rule table: native CPU (apportioned) vs facts CPU.

## Spike B: base + delta dataflow for cross-file rules

### Scope

Three cross-file rules, semantics taken from Jeremy's policies and ours, expressed over the exported facts:

- **R1 dead code**: `internal-dead-code` (definitions with no reference from any file in the worktree). Antijoin with counts.
- **R2 public API**: `public-api-triples`. Joins across files.
- **R3 reachability**: from the data-layer `calls` policies in `~/rerail-bench/data-layer/policies` (a function reaches a forbidden callee transitively across files). Recursion, demand-driven from the targets.

Where a rule depends on a fact B's extractor lacks, simplify and document. B's gate is incremental ≡ from-scratch, not oracle parity: the extractor is a stand-in for Phase 2's.

### Steps

1. **Extractor.** Tree-sitter Python walk producing the exported facts per content, persisted in the fact-file format. Extract the 7,091 distinct contents once (bench this separately; it is not part of the steady-state budget).
2. **Base and deltas.** Base = majority `(path, hash)` across the 40 worktrees. Each worktree's delta = paths where it differs. Report divergence per worktree (files, facts).
3. **Dataflow.** `differential-dataflow` + `timely`, one dataflow. Base relations arranged once and shared. Per-worktree inputs are deltas only. Non-recursive rules in base + delta form by hand (`Δ(R⋈S) = ΔR⋈S + R⋈ΔS + ΔR⋈ΔS`, counts for negation and aggregates). R3 demand-driven from the targets, materialized per worktree only when that worktree's delta touches call edges. Write down the expansion for each rule: Phase 2's compiler emits exactly that.
4. **Edit replay.** Deterministic seed, 1,000 single-file edits round-robin over the 40 worktrees: delete a function (25%), rename a function and its in-file references (20%), add or remove an import (15%), add or remove a call (20%), revert the worktree's previous edit (10%), whitespace-only edit, new hash and identical facts (10%). Plus 5 bulk events (200 files replaced with another worktree's content), 5 new-worktree events (clone of base, then 10 edits). If `~/repos/nera-data-layer` has history around `2b1504adc`, also replay 50 real commits as bulk events and report them separately.
5. **From-scratch oracle.** Naive Rust implementation of R1–R3 over `member(w) ⋈ facts` for a worktree, run on the edited worktree after every edit and on all 40 every 100 edits. Compare findings as sets, exactly.
6. **Steady-state measurement.** Start the process from facts on disk, load 40 worktrees, run the replay. Per-edit latency = input accepted → output frontier advanced. RSS at steady state. Per-worktree state: bytes attributable to each worktree's delta inputs and derived relations (DD trace sizes), tabulated against its divergence.
7. **Optional, one day cap**: the naive worktree-keyed variant (no base + delta) for R1, to quantify what base + delta saves.
8. **Report** with the template below.

### Gates

| Id | Gate | Target |
|---|---|---|
| B1 | Incremental ≡ from-scratch, every checked step over the whole replay | 0 differences |
| B2 | Per single-file edit latency, 40 worktrees loaded | p50 ≤ 10 ms, p95 ≤ 50 ms, p99 ≤ 200 ms |
| B3 | Resident memory at steady state, 40 worktrees, R1–R3 active | ≤ 200 MB |
| B4 | Per-worktree state | ∝ divergence; a clone of base ≈ findings only, not ∝ tree |
| B5 | New worktree (clone of base) / 200-file bulk change | ≤ 1 s / ≤ 2 s |

### Kill criteria

- B3 > 500 MB: implement the partially-ordered-time variant (one lattice bit per worktree) for the same rules and re-measure. Still > 500 MB: design idle-worktree eviction (membership is ~0.2 MB; rebuild ≤ 1 s) and revise the budget in the report.
- B1 fails: stop, root-cause, report. Exactness is not negotiable.
- B2 p95 > 50 ms only on R3: report R3 separately; demand-driven recursion design is revisited in Phase 2.

### Also report

- Divergence distribution across the 40 worktrees (files and facts per worktree).
- Exported fact volume: bytes per content, total resident for member hashes.
- Where DD's `reduce` sees keys touched by many worktrees (hot symbols), and what it cost.

## Report template (both spikes)

1. **Setup**: commit, build profile, machine load, corpus, what was excluded.
2. **Results**: one table, gate id → target → measured → pass/fail → run directory.
3. **How measured**: exact commands; what a fresh agent runs to reproduce each row.
4. **Parity or exactness artifacts**: paths to comparator output.
5. **Design notes**: final schema table; evaluator or dataflow choice; the delta expansion per rule (B); the stage census (A).
6. **Risks found**.
7. **Resume notes**: state of the branch, what is next, what is half-done.

## Rules of engagement

- Commit locally with prefixes `spike-a:` / `spike-b:`; do not push. Commit at every milestone (census, extractor, first rule, parity, speed) so a fresh agent can resume from the branch and the report; request budgets are finite.
- Code lives in the spike crate as a workspace member. Production crates are touched only for visibility changes needed to reuse the parser and `Facts`; no behaviour changes to the native engine or to Jeremy's rules.
- `CARGO_BUILD_JOBS=4`; own `CARGO_TARGET_DIR`; kill only your own PIDs; `bench-lock` around timing runs; no run over 15 minutes.
- Exact or refuse. No approximation anywhere, including in a "simplified" rule: simplify the rule's scope, never its semantics.
- No corpus content in commits. Tiny synthetic fixtures are fine.
- Delegation: `sonic` subagents for strictly mechanical work with a precise written spec; the executor runs and verifies everything they produce. Design, measurement and gates stay with the executor.
- Update your report file at every milestone, not only at the end. The decider reads it while you work.

## Review protocol (decider)

1. Read the report; check every results row cites a run directory and a command.
2. Re-run the parity or exactness comparator from the run directory once.
3. Re-run one speed row under `bench-lock` on an idle machine and compare with the reported median.
4. Record the gate outcome, the decision (go / shrink / kill), and the date in `SCRATCHPAD.md`.
5. Merge the spike branch into `Moti-Velz/facts-plan` only for its report and schema; the spike code stays on its branch.
