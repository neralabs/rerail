# 0028: G's grid executor is superseded; four of its ideas are kept

Status: accepted (2026-11-02, user decision); closes O33

## Context

Three Metal executors are in the tree:

| | N's Metal interpreter and JIT | G's grid executor (`ExecutionBackend::Metal`) | The wave (0024) |
|---|---|---|---|
| Unit of work | one thread per (program, file, anchor), backtracking over the bytecode | broad stages over a shared pool of match states, advanced generation by generation | label passes and micro walks over compacted start lists, in one command buffer |
| Size | ~1,300 lines | ~36,000 lines (`metal_grid*`, `metal_successor*`, `metal_terminal*`, `metal_frontier_*`, `metal_output_*`, closure, ready-batch, typed output) | ~4,500 lines with the work queue (0027) |
| On the audit path | the fallback 0024 names; roots stay on the CPU JIT (0017: Metal JIT roots compiled in 59 s against 0.8 s) | no (`UnsupportedBackend` there) | yes: 5,023 of S7's 5,028 (program, file) walks on the device |

G designed the grid for the scale corpus: 82.9 M (file, program, root) tuples over 9.6 M distinct roots (`tempo/docs/metal-production-grid-cutover-design.md`). G measured two failures of the task-per-thread model there:
- evaluation waves, which serialize a task's roots in one thread: 456 waves, 193 s;
- one atomic reservation per result: 47 s, against 3.05 s for count, scan and scatter.

G's answer was shared roots across programs, count/scan/scatter, and compact frontiers, with a target of under 40 launches and no host waits. G deleted N's interpreter so that the grid was the only executor.

The code does not reach that target. One frontier generation is 15–25 launches with about 6 host waits: prepare, count, exact allocation, publish, ready selection. So the 99-launch bound (list step 13) allows only 4–6 generations (O33). Moving those generations onto the device work queue (O33's proposed closure) would mean porting most of the successor pipeline into one kernel. The per-task barriers (readiness by minimum rank, duplicate removal, pending-offer ranking) would become threadgroup-local, and closure and inline predicates are multi-pass whole-frontier stages.

## The census (`slice::tests::pack_census`, 2026-11-02)

Every pack's rules and candidates, with the data-layer pack read from the Data-layer worktree. It covers each `forbid` `ast` matcher of each language, and each lowered program (root, and each label's) is tested with `Micro::of`:

| | Micro walks | Need a general executor |
|---|---:|---:|
| root programs | 1,318 | 6 |
| roots labels | 11,417 | 65 |
| targets, count, pattern, constrained | 2,839 | 0 |

So 71 of about 15,600 lowered programs (0.45%) are not micro walks. They are in react (`rules-of-hooks`), typescript (`explicit-function-return-type`), javascript (`no-unsafe-optional-chaining`) and data-layer (2 of 1,288).

The larger gap is coverage: 2,033 of 3,327 matchers (61%) do not lower at all. That includes data-layer's 217 of 383 and Python's 199 of 286. This is list steps 25–26 (semantic predicates as labels, O24), not an executor's.

## Decision

1. **The grid executor is not ported to the work queue and is not put on the audit path.** The wave's shape (0024) covers 99.5% of lowered programs on the device with far less machinery. The executor stays in the tree, as a copy, until list step 31 removes it with the copies it belongs to.
2. **What 0024 already took from G** (credited here): count, scan and scatter for outputs (G's 47 s against 3 s); work sized from compacted shared starts, not per-task records (G's 193 s evaluation waves; the ledger's "Tempo-metal's per-tuple task rows: about 16 days"); the corpus resident once (list step 28).
3. **Four ideas are kept for the general remainder.** If the 71 programs grow (data-layer's harder 82 checks, see below), a small device executor is rebuilt from them on the work queue (0027). It replaces both the grid and N's per-thread backtracking:
   - **Order by branch digits, not by row order.** Each state carries the branch choices that produced it (a digits arena). The terminal sort orders matches by (task, digits) and rejects equal positions (`metal_position_order.metal`, `position_compare`; `metal_terminal.rs` `encode_terminals`; `metal_terminal_publication_sort.*`). States can then be produced in any order: by a work queue, or with atomics. This is what ast-grep's first-match semantics need under breadth-first execution. A global `DISTINCT` loses NFA identity (DI, quantifiers: SCPR cat7 L614-675), and ordered siblings need an NFA (ledger #22).
   - **Duplicate active states are dropped by exact equality, keeping the minimum position** (`metal_successor_active.metal:40-80`). This bounds the state pool on repetitions.
   - **Closure by bulk scans** for quantified runs (`metal_bulk_closure.rs:205-316`, `metal_closure_continuation.rs`).
   - **Exact page boundaries** for output under a memory bound (`metal_output_pages.*`, `metal_output_selection.*`).
4. **Why not N's interpreter for the remainder:** generic interpretation is about 12× slower than specialized code (HI §2.23, S250). Per-thread backtracking is the task-shaped model G measured failing at scale. Acceptance must also be structural: work proportional to anchors and results (ledger #26).

## Scale check

The data-layer rules on the 9,010-file data-layer repository (137.9 MB) are the architecture's hardest test. The 60 ported checks are the simplest of 142. Their lowered programs are as micro-shaped as the other packs' (2 general of 1,288), but 217 of their 383 matchers do not lower yet. Before list step 30, the census is re-run as coverage grows, and point 3 is revisited if the general share rises.

## Consequences

- O33 is closed as superseded. O34 is unchanged.
- The five parity tests that skip the grid (`tests::grid`) keep skipping it until step 31.
