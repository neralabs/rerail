# Facts engine: plan

Status and results live in `SCRATCHPAD.md` (updated by the decider only). The current phase's detailed spec is `PHASE-1.md`.

## Goal

Run hundreds of complex data-layer rules as edits go out, across ~40 parallel worktrees, exactly, under 1 GiB resident, with per-edit latency ≤ 100 ms for per-file findings and ≤ 500 ms for cross-file findings. Per-edit cost must grow with the edit, not with the rule count.

## What we measured (the numbers this plan rests on)

| Fact | Value | Source |
|---|---|---|
| Duplication across 47 worktrees | 262,567 `.py` files, 7,091 distinct contents (37×) | memory47 |
| Batch 6 (10 heaviest ported rules), native, one tree | 323 s wall, 393 s CPU, 28.6 GiB | dl60 |
| Same, Python oracle (4 workers, no fact cache) | 23.4 s | dl60 |
| Batch 6 global phase (single thread) | 267 s = 198 s SQLite stage builds (164,742) + 45 s input re-extraction (81,532 calls) + 13.5 s setup + 1.8 s producer | dl60-split |
| Saved by building each (stage, file) once | 0.08 s. Inputs differ per clause by gate and projection | dl60-split |
| 28.6 GiB cause | all 11,044 per-file jobs collected (9.85M rows with JSON maps) before any SQL runs | dl60-split |
| Edit cost today (first 50 / batch 5 / batch 6) | 50.6 s / 20.2 s / 12.3 s | jb-expressivity, dl60 |
| Native per-file matcher cost on his rules | 0.4–1.4 ms per rule per file | dl60 |
| Jeremy's `Facts::build` (shared syntax facts) | 27 ms per MB of source | his docs |
| Store compaction (done, merged) | native 8 worktrees cold 204→93 s, warm 114→14 s; 2.0M result rows → 17k rows + 140k words | store-compact |
| SQL set-composition engine (done, parked) | exact; ~3× native cold; never faster on Jeremy's rules; loss was SQLite per-statement and B-tree cost | loop, dl60 |

## Architecture

**SQL in, facts underneath, one algebra in between.** The binding statement is `INVARIANTS.md` (I1–I10); this section is its summary.

1. **One logical algebra with SQLite's semantics**, produced by two front ends (SQL stages, pattern DSL). After the parser nothing holds SQL text or stage names: no shape-recognized substitutions, no kernels selected by name, no per-operator fast paths. Speed comes from general, justified rewrites (`REWRITES.md`) and a columnar physical algebra planned once per rule.
2. **Facts are relations with declared structure** (keys, functional dependencies, preorder order and its interval property); derived relations are SQL views over the syntactic core, compiled like any rule.
3. **Classification is a type:** per-content iff every leaf is per-content; the declaration is checked, never trusted; a per-content plan physically cannot reach membership relations.
4. **Memoization by expression digest** `(content hash, schema version, plan digest)`, fleet-wide; **incrementality generated** from the algebra for membership-dependent plans (per-path base, cluster bases, worktree deltas; state ∝ divergence).
5. **One daemon;** per-worktree state = membership + deltas + findings; hash-first edit protocol; findings as epoch-stamped deltas.
6. **Exact or refuse; admission by gate.** Every rewrite, operator and generator change lands with its differential test and mutant.

## Targets

| Metric | Today (60 rules) | Target (300 rules) |
|---|---|---|
| One-file edit → per-file findings | 12–50 s | ≤ 100 ms p95 |
| One-file edit → cross-file findings | same rerun | ≤ 500 ms p95 |
| New worktree from main | minutes | ≤ 1 s |
| Fleet RSS, 40 worktrees | 2.5 GiB (light rules); 28.6 GiB (batch 6, one tree) | < 1 GiB |
| New or edited rule, fleet-wide | rerun per worktree | ≤ 10 s, background |
| Correctness | oracle per rule | oracle per rule, plus incremental ≡ from-scratch on replayed edits |

## Phases

Each phase has a gate (numbers measured before the code is written) and a kill criterion (what we do instead if the gate fails). A phase exits only when the decider records the measured gate in `SCRATCHPAD.md`.

### Phase 1: de-risk with two spikes (running)

- **Spike A: facts once + in-memory relational evaluation**, on batch 6 (the 10 heaviest rules). Gate: oracle parity; ≤ 10% of native CPU for the same rules (≤ 40 s vs 393 s); ≤ 0.5 ms per clause per file; ≤ 2 GiB peak. Kill: if < 4× win with parity, the in-memory evaluator replaces only the SQLite stages and pattern matching stays native.
- **Spike B: base + delta dataflow for cross-file rules** on 40 of the 47 worktrees, 1,000 replayed edits. Gate: incremental ≡ from-scratch at every step; p95 ≤ 50 ms per edit; ≤ 200 MB resident; per-worktree state ∝ divergence; new worktree ≤ 1 s. Kill: > 500 MB → partially-ordered-time variant; still failing → idle-worktree eviction and a revised budget. Exactness failure is a stop.

Exit: both gates measured; go/no-go written in `SCRATCHPAD.md`.

### Phase 2: compiler

Rule IR; SQL-fragment parser (SELECT/JOIN/WHERE/GROUP BY/UNION/recursive CTE with set semantics); classifier; delta-rule emission for non-recursive strata; demand-driven recursion; fact schema v1 with versioning; census and plan audit; from-scratch reference evaluator; the gate (incremental vs from-scratch on recorded edit sequences, with seeded mutants in the delta rules). Gate: all 60 ported rules compile and classify; a seeded cross-file leak is rejected; batch-6 parity through the compiled path.

### Phase 3: store and daemon

Per-content facts and findings in the content store; one daemon; hash-first socket; Git ODB bulk sync; per-worktree membership; delete per-worktree resident documents and the collect-all-jobs global phase. Gate: 40 worktrees under 1 GiB on the memory47 harness with the 60 rules; new worktree ≤ 1 s; per-edit targets met.

### Phase 4: port at scale

Recorder and replay per rule (exist); LLM-assisted porting with the oracle as the test; weekly census; per-rule µs per file tracked so the per-edit budget holds as rules land. Transitional: unported per-file Python linters memoized per content in a worker pool. Gate: per-rule oracle parity; per-edit budget holds at the rule count reached.

### Phase 5: cutover

Budget-exceeded contents to a background unbounded lane, cached; retire the SQLite global phase and the SQL executor; keep the gate. Gate: production path on the facts engine with the memory47 harness green.

## Roles

- **Decider and verifier**: this session. Reviews reports, re-runs gates, keeps `SCRATCHPAD.md`, records go/no-go. Does not write engine code.
- **Executors**: Opus 5.5 `task` subagents, one per spike or slice, each in its own worktree and branch. They may hand strictly mechanical generation (translations with a precise spec, fixture munging) to low-reasoning `sonic` subagents and must verify the output by running it. Design, measurement and gates are never delegated.
- **Jeremy**: owns the relational executor and the rules. Replacement of his executor is sequenced with him; rule semantics are never changed by us.

## Decision log

- SQL set-composition engine retired as an executor: exact, but ~3× native on our rules and never faster on Jeremy's 60 (dl60). Native stays the production path until Phase 5.
- Phases renumbered: the two spikes are Phase 1 (previously "Phase 0").
- Base for Phase 1 work: `Moti-Velz/sql-dl60` at `e6d2ac73c` (integration + Jeremy's expressivity branch + engine switch + predicate fix).
