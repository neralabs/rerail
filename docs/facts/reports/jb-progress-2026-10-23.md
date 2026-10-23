## Spikes branch

**NOT COMPLETED: blocked on tooling. No findings below are observed.**

The scout had only `read`, `grep`, `glob`, `web_search` and `write`. It had no shell, so `git show` and `git worktree add` could not run.

Tried:
- `~/orca/rerail/.git/objects` holds zlib-compressed objects and `origin/jeremy/sql-capture-demand-spikes` is only a ref, so the content is not readable.
- No checked-out copy exists. `jb-sql-spikes` is gone from `~/orca/workspaces/rerail`. The five new docs (`sql-relational-architecture-review.md`, `sql-relational-transition-plan.md`, `sql-set-composition-compiler-spike.md`, `sql-generation-shape-spikes.md`, `sql-work-amplification-spikes.md`) do not exist in any worktree or under `/tmp`.
- Raw GitHub (`neralabs/rerail`, private) returned 404.
- Grepping the omp sessions found no earlier dump of these docs.

The only usable prior material is `local://jb-spikes.md`, which covers up to `b4ea853cd`. It says nothing about `605ed842c`, `2aca38931` or the 25 new commits.

Needed to finish: a session with shell access, to run `git -C ~/orca/rerail worktree add --detach /tmp/jb-spikes origin/jeremy/sql-capture-demand-spikes`, or to paste the five docs and `git log b4ea853cd..origin/jeremy/sql-capture-demand-spikes`.

---

## Rerailer branch

Sources: `/tmp/jb-rerailer` (= `origin/jbarnes/ner-1562-sql-rerailer`, tip `7db44ee47`). Tip is the same one `local://jb-rerailer.md` reviewed, so the earlier measurements still stand. Commit `f352a1d6d` was not readable (no git); "phase six complete" is taken from the docs. Marked [INFERENCE] where it is mine.

### Status
- **"Phase six" = Phase 6 of a 10-phase SQL conformance remediation, not a product phase.** `docs/sql-conformance-remediation.md:726-743`: "Phases 1 through 6 are complete." Phase 6 lowers file regexes, `select_when`, `require_each`, max/distinct/grouped budgets into completion-guarded SQL. File language/path/module/required-text conditions prune tasks before terminal demand. Syntax-error absence stays "unknown". Producer failures are batch-local.
- **What "complete" means: authored SQL and ordered-catalog goldens plus matcher-oracle checks.** The first line is 39 Phase 2–4 contracts and 4 Phase 5 and 10 Phase 6 contracts (`:693-695`, `:740-743`). Each passes "exact, behavioral, repeated, concurrent, and independent low-level digest checks". These are 108-form hand-written fixtures, not real policies.
- **Open (his own doc):** Phase 7 violation extraction (`:982`), Phase 8 packed relation and immutable demand (`:1007`), Phase 9 workspace prep and publication (`:1056`), Phase 10 full 108-case conformance and performance (`:1095`). `docs/sql-relational-compiler-design.md:28`: "workspace tape-preparation and result-publication stages remain incomplete."
- **The full scan does NOT run on the SQL design.** `sql-relational-compiler-design.md:670-676`: "There is no valid post-cutover full-scan result. The exact data-layer scan stops during whole-matcher packed lowering before SQLite execution. The last 17.189-second full scan measured the deleted matcher-backed path... Neither result measures this design." Same as our last review. Code returns "not implemented" for packed workspace tape/task preparation and publication (from `local://jb-rerailer.md`, `evaluate.rs:84-121`, `api.rs:806`; not re-opened today).

### Measured numbers (all carried from the branch; none new)
Workload: data-layer `fce4dc72`, 12 policies, 829 clauses, 5,725 files, 95.7 MB, M5 Max, 4 workers.
- Historical callback SQL: 1,404.7 s, CPU 2,199.9 s, 15.1 GiB; 18,264 violations.
- "Current" 17.189 s, CPU 53.5 s, RSS 3.47 GB — this is the **deleted matcher-backed path**, not SQL. Matcher control 8.983 s (14,656 violations); the SQLite route returned +318 violations, so parity was not claimed (`benchmark/history/ner-1562-component-compiler-and-real-catalog-2026-10-18.md`, per earlier review).
- Compiler: 16-thread 1.46 s wall for the real catalog.
- Packed JIT/Metal backends gave no speedup on selective work (`docs/transducer-design.md:302-320`).
- **Nothing new since 10-20: no SQL full-scan number exists, so nothing to compare with our 90 s CPU / 28 s wall / 3.1 GB / 173/173.**

### Exactness method
Matcher oracle: 108 authored operator forms with oracle outputs, plus SQL-shape and terminal-catalog goldens (53 statements, `sql-conformance-remediation.md:692-702`). Intent is "complete SQL outcomes equal matcher-oracle outcomes" (`sql-relational-compiler-design.md:666`). **No real catalog has been shown to lower and match.** Last real-catalog comparison was +318 violations.

### Cross-file / incremental
- Cross-file design: "File-local programs emit evidence and completion rows in the same demand. SQL performs deterministic cross-file joins, anti-joins, budgets, and thresholds. Incomplete evidence cannot prove absence." (`sql-relational-compiler-design.md:435-441`). Phase 6 lowers `require_each`/`budget`/`select_when` in this shape, at fixture level.
- Incremental/invalidation is only a plan: revival design Phase 4 "Add deterministic producer and consumer invalidation. Prove fresh, incremental, restored, and evicted parity" (`sql-rerailer-revival-design.md:497-504`); gates require incremental p50 ≤100 ms, p95 ≤500 ms, RSS <1 GiB (`:534-536`).
- **No worktree, membership, or per-worktree state anywhere in these docs** (grep for worktree/Moti/facts hit nothing relevant). Cross-file here means rule-level cross-file clauses within one workspace, not many checkouts sharing state. His multi-workspace sharing lives in the landing content-addressed store (see `local://jb-landing.md` §3), not the rerailer branch.

### Relevance to our gates
- **Per-file G2.3-class:** none yet. He has no SQL full-scan result, no parity run on a real catalog, and no throughput figure for the SQL design. Our batch-6 numbers (parity 173/173, 90 s CPU) are ahead of anything he has measured on this path. His design still has the row-amplification risks listed in `local://jb-rerailer.md` (edge closure, universe anti-join, JSON bindings).
- **Cross-file G2.4-class:** he has a design and fixture-level SQL for the clause kinds; ours has a linker exact on differential-dataflow with 4 of 8 clauses exact plus a 40-worktree memory figure. He has no incremental or multi-worktree measurement. [INFERENCE] His cross-file approach (evidence rows + SQL joins, recomputed per demand) would face the same per-demand recompute cost that incremental maintenance avoids.

---

## Landing (Metal Strip)

Sources: `/tmp/jb-landing/tempo/docs/metal-execution-cutover-{plan,handover,phase3,phase4}.md`, `metal-query-greedy-contract.md`, `benchmark/history/ner-1562-phase4-strip-2026-10-23.json`. Tip `f1af1a49`. Commit hashes cited are from the docs.

### What Strip is
**Strip is not the ordered-GREEDY closure.** It is the `#strip!` output directive of the tree-sitter query dialect: delete every regex match from a capture's text. On Metal it is an ordered regex find over an NFA frontier, then a byte-deletion mask and a scan over 256-byte tiles to compact output (`metal-execution-cutover-phase4.md:6-34`; `transducer-value-program-design.md:307` "repeated regex search and TEXT_COPY_EXCEPT_MATCH"). It is the last of seven output instruction kinds to get a GPU implementation (`phase4.md:8`). It is phase-four work (shared predicates and typed output), so it is part of making every Program type run on the GPU, not a speedup.

### The ordered-GREEDY blocker (the thing that blocked him before)
It is resolved separately, before Strip:
- Diagnosis: 128 and 1,024 siblings need 128 and 1,024 header rounds, which exceeds the 100-launch limit (`phase3.md:878-929`, in the earlier review).
- Fix: a **bulk-closure operator** for capture-free terminal GREEDY loops (`phase3.md:910-925`). The user approved a CPU prototype first (`:968`), then a Metal implementation. Tests `metal_bulk_closure_tests::gpu_bulk_closure_*` and `ordered::bulk_closure::tests::*` are in the Strip receipt's test list.
- Companion decision: the query-GREEDY contract (`metal-query-greedy-contract.md`, 2026-10-21) rejects unsupported forms at construction.
- Phase 3 is declared complete (`plan.md:405-432`): 358 tests, 355 pass, 3 opt-ins; 5-match optional query 234 launches; captured repetitions at 2/33/130 siblings use 13/18/20 launches; max production count 596.

### Is Strip exact?
Exactness is against **independent CPU oracles and manual truth**, not against production data:
- Red-first: six red commits (`cf6024e39`, `047ddd43c`, `c77de0cad`, `5e7b694ed`, `fd2510ba6`, `fdfa39fbf`).
- The oracle audit found four defects in the **Native CPU oracle itself** before connecting the GPU (anchor origin moved after each match, empty matches dropped prefixes, shared work counter, undercharged unanchored NFA work) (`phase4.md:19-27`). So "oracle repair" means fixing his reference implementation, and parity is "GPU = repaired CPU".
- Fixtures: 16 manual text cases, 19 saved-continuation cases, 12 invalid-producer controls, 6 Programs × 1 and 128 files; all ten retained Strip fixtures pass (`phase4.md:44-48`).
- **"Exact candidate"** = exact per-candidate transform *budget* boundaries (640,022-unit boundary, `phase4.md:41`), not an exact-set claim.
- **"Oracle regeneration"** = a generated-source digest guard: the full run first passed 705/706 and failed only the generated experiment digest, then `60278619b` regenerated it; "its only generated change is the recorded oracle digest" (`phase4.md:68-70`). It is a bookkeeping hash, not a semantic oracle rebuild.
- Full native suite: 706 pass, 0 fail, 22 ignored, 467.58 s; peak RSS 1.30 GB (receipt).
- `phase4.md:51`: the catalog check compiles all 335 pinned patterns, "does not establish complete corpus parity".

### Measured numbers vs CPU
Fixtures only, and he disclaims them: "These fixture times do not establish a corpus speedup" (`phase4.md:59`).
| Case | Metal | Native CPU | Launches | Metal bytes |
|---|---:|---:|---:|---:|
| 6 Programs, 1 file | 0.626 s | 0.732 s | 69 | 8,404,992 |
| 6 Programs, 128 files | 0.397 s | 0.064 s | 94 | 8,486,912 |
- At 128 files Metal is about 6.2× **slower** than native CPU (computed from his two figures; he does not say it). The prior GPU path refused Strip, so there is no baseline.
- Largest logged launch count across fixtures is 595; max Metal allocation 637 MB; the cap is still the **temporary 600** launches (the real target is <100, "later optimization... remains open", `phase4.md:76`).
- No corpus number from this track exists. The last corpus numbers remain those of the earlier `local://jb-landing.md` §2 (the 40.04 ms spike does not implement Program semantics; deleted scheduler 1,978,757 launches).

### Status and path to rerail
- Phase 4 of 5 is incomplete: "Saved GPU output pages and remaining phase-four acceptance still require completion. Phase five has not started." (`plan.md:10-11`). Phase 5 = full pinned corpus GPU parity, persistence/reopen, zero-legacy acceptance (`handover.md:106`). The "879 corpus matches" regression is not yet recovered (`handover.md:105`).
- **Research-to-production path, but not into rerail's engine.** The track is the production Metal executor for *tempo/landing* transducer Programs (tree-sitter query syntax). Earlier review: the rerail rule DSL is not lowered to Programs (`AstMatcher::Program` is dead code outside tests). The rerailer branch's SQL design would consume the packed backend as a terminal producer (`sql-conformance-remediation.md:755-763`), but its terminals are wordcode-only today and AST patterns still run on CPU. [INFERENCE] So the Metal work is a terminal backend that might later sit under his SQL rerailer; nothing documents that wiring or a schedule for it.
- The only rerail link in the landing docs is `rerail-observable-requirements.md` (survey 2026-10-21): R5/R6 (stable findings across incremental updates, retraction on edit) and R9 (latency/RSS budgets). It states no rerail parity is established for repository-wide analyses (`:91-92`).

### Relevance to our gates
- **Per-file G2.3-class:** none. It does not address SQL row amplification. It is GPU stage plumbing, with the GPU slower than CPU at 128 files and 69–595 launches versus a target of <100. Nothing here changes the comparison with our 90 s/28 s/3.1 GB/173-of-173.
- **Cross-file G2.4-class:** none. The track is single-file structural matching and output transforms. Its multi-workspace sharing (content-addressed store, per-(blob,catalog-check) results, deleted at HEAD per `local://jb-landing.md` §2.4) is older work.

---

## Across both: cross-file, multi-worktree, and intent
- **Cross-file / multi-worktree:** neither branch targets incremental maintenance, membership, or per-worktree state for cross-file facts. The rerailer branch has SQL-level cross-file clauses over one demand and an unmeasured Phase 4 invalidation plan. The landing branch is per-file throughput. His content-addressed per-blob reuse across worktrees (old work) is for per-file results and explicitly excludes global/cross-file checks (`content-addressed-workspace-store.md:54`, from the earlier review).
- **Intent toward our work:** I found no statement about merging, replacing, or ignoring the facts engine in either set of docs. Grep for `Moti|facts engine|replace|merge` in the rerailer SQL docs and the landing Metal docs returned only unrelated hits. One related line: the rerailer design says "The current dirty Landing worktree is research input only. Ports must use named clean commits and must include their parity tests" (`sql-relational-compiler-design.md:558-559`). Docs record his plan to rebuild SQL on his own packed backend (see `sql-rerailer-revival-design.md`). Chat or PR statements are not visible to me; GitHub shows no PR from him in either repo (`pr://` lists, 2026-10-23).
- **Not covered:** the `jeremy/sql-capture-demand-spikes` and `jeremybarnes/Data-layer-rule-expressivity` branches (see the spikes section above) and anything in git history I could not read (commit messages incl. `f352a1d6d`, `f1af1a49`).
