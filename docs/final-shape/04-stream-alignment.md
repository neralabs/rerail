# Stream alignment against the target architecture

Status: 2026-10-26. This compares each stream's current work with `03-architecture.md` (§ numbers below refer to it).

Sources:
- DLRE, N1562, PIAT, SSE, Typing and Tempo were re-read at their HEADs today.
- Moti's branches and SSC are unchanged since `notes/t10` and `notes/t11`.

M marks a measured figure; A marks an estimate.

## The pattern across streams

- **No stream is building §1–§2:**
  - the fact lattice in SQLite;
  - the push path with exact-same cutoff;
  - the overlay with background write-back.

  Every engine stream throws its store away after a run: PIAT's `Store::temporary`, SSE's in-memory facts DB, DLRE's execution-scoped data, N1562's TEMP packets. So no stream has a warm path. Moti's Y1 and Typing's interface digests have the cutoff mechanism, but outside SQLite.
- **Three streams still evaluate tree matching in SQL inside documents:**
  - PIAT runs `inside`/`has` as CTE stages;
  - N1562 uses recursive CTEs over navigation edges, with most of its effort going into fitting SQL text under 8 MiB;
  - DLRE runs per-scope family SQL.

  §3/§7 put this in combined transducers. Only Tempo runs it on the GPU, and Tempo is in path mode, uncombined, and copies its inputs.
- **Every stream is over 500 MB:**

  | Stream | Memory (M) |
  |---|---|
  | Typing | 445–724 MB |
  | SSE | 1.3 GB |
  | Moti Y3 | 2 GB |
  | Tempo | 2 GB Metal + 3.5 GB RSS |
  | DLRE | 4.7–6.5 GB |

  None gates on the budget.
- **Construction dominates where it is measured:**
  - PIAT: 131 s of construction against 3.3 s of matching;
  - SSE: 54.8 s against 0.35 s.

  Both pay construction again on every run, because nothing persists.

## Per stream

| Stream | Doing now | Keep (maps to) | Conflicts | Steer to |
|---|---|---|---|---|
| **PIAT** | Turning anchor plans into SQL stages: set algebra, vocabulary fill, text fallback, `nera_tree` neighbourhoods. Dead-code anchor selection on the full corpus: 68.7 → 21.5 s (M) | §6 prefilter over always-built kind and term postings; `nera_tree` (§7 tree as tape arithmetic) | in-document relation SQL; close to per-file replay batches; temporary store; choices structural rather than statistical | **§6 owner.** Anchor SQL does prefiltering only (file, kind, term → candidate set). The store becomes a persistent fact family with `compiled_plans`. Anchors are chosen from statistics. `inside`/`has` move behind `demand(batch)` |
| **N1562** | Matcher → typed SQL compiler: CTE fencing, region packets, fitting under 8 MiB (`async_sync_delegation` 8.55 → 4.83 MB, M); 7 policies blocked | the typed packed vtab and `PackedDemand`, one per batch (§3 vtab contract); kind probes ordered by `search_kind_vocab` | tree axes and recursive utilities as SQL CTEs; no fact caching; no CPU numbers | **Stop SQL-size work.** Its vtab becomes `demand(batch)`. `has`/`inside`/`stopBy` and bounded utilities move into the transducer program. SQL keeps cross-clause joins only |
| **DLRE** | Schema lifecycle SC1/SC2: shared family programs, one `ANALYZE` per batch, family frontier before readers. Full run in progress. Control: 743.5 s / 4.74 GB (first-50), 3,899 s / 6.52 GB (all-60) (M) | the statistics-in-SQL discipline; data-layer rules and corpus as the **acceptance gate** | per-scope execution; nothing persists; 4.7–6.5 GB | **Becomes the gate, not an engine.** Finish SC1, then freeze new mechanisms. Its rules and corpus measure the new path (cold CPU + RSS ≤ 500 MB + finding keys) |
| **SSE** | Native producers → narrow `py.*` facts → vtab → 2 SQL statements → finding keys, for the ASYNC21x family. Full corpus: 350 ms SQL (M); demand cuts requests 65,302 → 12 (M) | closest to §4/§8/§9: facts via vtab, demand-installed facts, one consolidation; a template for the **findings layer** | producers scan 19M rows with no prefilter; facts in memory per run; 1.5 GiB ceiling; CTE fixpoints | **§4/§8 owner.** Producers are fed from anchor postings. `py.*` become versioned SQLite facts with cutoff (Phase 5 brought forward). Gate at 500 MB |
| **Typing** | Round 8: magic sets over the whole program, so rules no longer carry demand. Generated demand matches hand-written (2.53M vs 2.51M tuples, M), but a one-file query costs 1.64–1.83 s (M), half a full run | the Datalog/lattice rule compiler (§0), semi-naive waves, magic sets as the **"install only needed facts"** mechanism, stats-driven replanning | whole-program evaluation in memory; 445–724 MB; no persistence or cutoff in program mode; the per-file split of `calls.rdl` not built | **§0/§2 generic-rule owner.** Split file-local strata (in-GPU candidates) from cross-file strata (waves). Persist per-file partitions with digests in SQLite. Measure body and interface edits (CPU, re-runs vs changes, RSS). Magic sets choose what each wave installs |
| **Tempo (NER-1562 GPU)** | Getting the 335-pattern CPython run (2,079 files) under the 600-dispatch cap: 19 successor generations = 380 dispatches; widening proved repeat jobs (uncommitted). 506 s wall, 2 GB Metal (M) | batches of documents × programs; bounded matcher recursion on GPU; every op on Metal; no fallback | path mode with exact capture parity; no cross-rule combining; synchronous MSL compile; tapes copied per call; whole-corpus scale | **§3/§5 owner.** Set-mode oracle (finding keys + Complete/Unknown). Hash-consed cross-rule graph. Interpreter first, background JIT, same plan identity. Pinned no-copy slabs plus a program cache prepared per wave. Gate on a prefiltered wave inside 500 MB |
| **Moti (facts-engine)** | Unchanged: Y3 wired path, exact on 1,151 edits, 0.53 / 3.76 / 8.62 ms (M), 2 GB heap | Y1 keyed tables + read-set memos with cutoff and backdating, the closest existing **push-path** design (§2); the mutant gate | SQLite replaced; 2 GB; Y2 level witness (not needed: we recompute) | **§2 design input.** Port Y1's read-set and cutoff scheme onto SQLite-backed facts with an overlay |
| **SSC** | Unchanged: transition plan from matcher continuations to shared plans | plan records (`RelationSpec`/`Stage`/`ProgramGroup`/`Plan`) as **plan identity**; the findings and coverage contract; 19 lessons | retire the cohort interpreter (already planned) | merge the plan records into `compiled_plans` |
| **Main** | shipped content store, ast-grep matcher per file | CAS, snapshots, leases, the positional kind FTS in one store, `calls` stamps | per-file matcher (kept as the rehydration oracle) | the home of the §1 fact store |

## Missing owners

1. **The §1/§2 fact store and push path.**
   - Scope: the fact schema in the content store, versions, exact-same cutoff, the overlay, and a background writer with back-pressure and spill.
   - No stream builds it, and every warm-path target depends on it.
   - Inputs:
     - main's CAS (home);
     - Moti's Y1 (cutoff design);
     - Typing's interface digests;
     - SSE's `py.*` facts (first customer).
2. **The wave scheduler (§4/§7).**
   - Scope: SQL-driven waves that call `demand(batch)`, prepare the GPU cache, and loop until there is no delta.
   - N1562's `PackedDemand`, DLRE's family frontier and SSE's 2-statement plan are each partial versions of it.

## Order

1. **Freeze the work that conflicts:**
   - N1562's SQL-size work;
   - PIAT's in-document relation SQL;
   - DLRE mechanism work after SC1;
   - Tempo's capture-parity generation work.
2. **Stand up the missing fact store and push path** on main's CAS, with SSE's `py.*` facts as the first relation family. Measure:
   - a body edit and an interface edit;
   - exact-same cutoff hits;
   - RSS within 500 MB.
3. **Tempo moves to set mode and the combined graph behind `demand(batch)`** (N1562's vtab contract), with pinned slabs.
4. **PIAT's prefilter feeds the batch;** Typing's split feeds file-local strata and waves.
5. **DLRE's data-layer rules gate the result:**
   - cold 1–2 s;
   - warm ms;
   - 500 MB.
