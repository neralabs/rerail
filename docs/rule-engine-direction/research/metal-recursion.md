# Data-dependent recursion on Metal

> **Superseded (2026-10-25).** This report predates `metal-enqueue.md` and the no-fallback rule (spec §8). Statements below that route anything to a CPU/JIT rerun are replaced: monotone recursion runs on Metal as GPU-scheduled waves, constructs that cannot be stratified are rejected by the verifier, and hard device limits yield `Unknown(resource)`, which is counted and gated.

Sources and raw output are in `../experiments/metal-recursion/` (`gen.py`, `kernels.metal`, `bench.swift`, `run1.txt`, `run2.txt`, `runc.txt`).

## Conclusion

Do not run tree recursion as recursion on the GPU. A preorder tape with a subtree-size column turns every tree relation into range arithmetic over sorted node sets:

- **`has` / `DESCEND`:** the subtree is the interval `[r+1, r+size[r])`.
- **`stopBy`:**
  - prune by skipping a subtree (`j += size[j]`);
  - or check reachability with a nearest-pruning-ancestor column `anc_P`. A hit `h` is reachable from root `r` iff `anc_P[h]` is NONE or `≤ r`.
- **`inside`:** a parent walk bounded by the batch's maximum depth.
- **Siblings:** skip pointers.

**Nested sub-rules become strata.** This covers `EXISTS`/`NOT_EXISTS`, `stopBy: <rule>`, nested patterns and non-recursive utilities.

- The inner rule is materialized first as a sorted node set, either a posting list or a bitmap.
- The outer relation then becomes a range query or set operation against that set.
- Dispatches are O(rule nesting depth) per batch. They are not O(tree depth), and not O(instructions × generations).

**What still recurses.** Only monotone, single-direction recursive utilities need O(depth) waves, and those are driven by the GPU inside one command buffer.

**Fallback.** These get `Unknown`, then a CPU/JIT rerun for that (program, document) only:
- anything the verifier cannot stratify;
- any document over the batch limits.

## Why the Tempo and N1562 executors hit their guards

The cause is not recursion. It is three things:

1. **Path enumeration.** `DESCEND` reads the descendant range (`metal_step_helpers.metal:36`), then forks one frontier state per descendant.
2. **One instruction per generation, under exact ordering and containment.** Each generation costs 44 launches. A two-node fixture hit the old 200-dispatch cap at generation five.
3. **Quadratic capture-containment antichains.** The full corpus reached 584 dispatches and 1.91 GB tracked, then failed on 21.7M comparison pairs.

N1562's runtime uses per-path pending, capture, cursor and repeat arenas with atomic quotas. Its memory scales with live paths × state.

Set semantics (decision 0001) removes the containment and ordering costs, and the range formulation removes path enumeration. Tempo's own shape spike already showed that broad grid kernels with count/scan/scatter work: 34 invocations and 40 ms at production volumes.

## Technique verdicts

| Technique | Verdict | Evidence |
|---|---|---|
| Interval arithmetic (subtree = range) | Adopt as the base formulation | Measured: no stack, sequential reads |
| Posting lists + range search | Adopt for selective sets | Measured: 20–2000× faster than any traversal; same postings as the kind index |
| `anc_P` / parent columns | Adopt | Measured: 0.14–0.63 ms to build over 5.4M nodes |
| SIMD-group cooperative scan | Adopt for huge intervals | Measured: fixes load imbalance (10.6 → 0.86 ms) |
| Indirect, GPU-driven dispatch | Adopt wherever sizes are data-dependent | Measured: 70 dispatches in one command buffer, no CPU round trips |
| BFS frontier waves | Keep for recursive utilities only | Measured: correct but 10–100× slower |
| Explicit per-thread stacks | CPU/JIT only | Measured: overflowed at cap 64 on 25–467 roots per query |
| Persistent-threads queue | Reject | Metal gives no forward-progress guarantee within a SIMD group. Spinning lanes starve same-group lanes that hold work. Results were correct only because the spin cap broke the waits. With a cap of 2^24 the run did not finish in 10 minutes |
| Continuation/requeue | Equivalent to BFS waves | Not measured |
| Frontier sort/compaction | Only inside count/scan/scatter | Sets stay sorted by construction |
| Depth bound + CPU flag | Adopt as the fallback | `Unknown` per (program, document) |

## Design: stratified range execution

**Resident data per batch:**
- `kind` (u16);
- `size` and `parent` at a fixed width per document;
- an optional `field`/`flags` byte;
- the document table;
- kind postings from the index;
- optionally a text-hash column for capture equality.

There is no `MetalNode` expansion and no CSR child list.

**Operators over node sets:**

| Operator | Execution |
|---|---|
| Leaf | Kind postings |
| Filter (local tests, text, regex) | Node-parallel, then count/scan/scatter |
| `has`, `stopBy: end` | Two binary searches per root, or a merge-join when both sides are large |
| `has`, `stopBy: P` | Range plus an `anc_P` check, or a SIMD scan for huge roots |
| `has`, `stopBy: neighbor`/child/field | `parent[s] ∈ R` |
| `inside` | Bounded parent walk |
| `follows` / `precedes` | Sibling range plus parent equality |
| `nthChild` | Sibling walk under the parent, or an ordinal column |
| `all` / `any` / `not` | Sorted merges or bitmap operations |
| `count` | Range length, or a prefix-sum range sum |
| Capture equality | Join on `(root, text_hash)` |
| Utilities | Shared DAG nodes, deduplicated by digest, evaluated once per batch |

**Recursive utilities:**
- Only monotone recursion in one direction is admitted: `has`-only or `inside`-only, with no negation or count through the recursive edge.
- `has` recursion runs as depth-level waves from deepest to shallowest, with one evaluate dispatch and one scan dispatch per level.
- `inside` recursion runs top-down.

**Dispatch shape:**
- Dispatches = strata × a small constant, plus 2 × max depth for recursive utilities.
- All of a batch's programs are grouped per stratum, with threadgroups uniform by signature.
- Counts come before allocation (count → scan → scatter). Indirect arguments are written by GPU prep kernels.
- The whole plan runs as one command buffer, and its dispatch count is known before execution.

**Memory:**
- Every set is bounded by the batch's node count.
- Leaf bounds come from kind statistics. Bounds then propagate through the operators.
- The arena is sized at plan time from the sets that are live at the same time.
- Each set is either a posting list (4 B per element) or a bitmap (n/8 bytes), chosen from its bound. A dense bitmap over 5.4M nodes is 675 KB.
- There are no per-path or per-thread worst-case arenas.

**Fallback.** Each of these yields `Unknown`, followed by a CPU/JIT rerun with explicit stacks:
- a document over the limits;
- an arena bound that doesn't fit;
- a construct flagged `CPU_ONLY`.

**What the verifier enforces:**
1. The rule graph is acyclic, apart from monotone single-direction recursive groups.
2. Each operator has capability flags, and a program's capability is their intersection.
3. Strata, live sets and arena size are bounded at plan time.
4. Set semantics only.
5. Every loop is bounded by a batch constant: depth, fan-out or range.

**Relationship to SQL.** The node sets are the SQL layer's own objects: kind postings and interval containment. The same plan therefore lowers to pure SQL through `fts5vocab` offsets and interval predicates, which is ast-index-spike's approach.

## Measurements

**Setup:**
- Apple M5 Max, 40 GPU cores. Shaders were compiled at run time, because the offline Metal toolchain is not installed.
- Corpus: the CPython 3.15 stdlib, 2,049 files, parsed with Python `ast` into one regular tape. That gives 5,396,512 nodes, maximum depth 34.
- Times are GPU medians of 7 runs, with buffers resident and upload excluded. Results were validated against a CPU reference.

**Caveats:**
- This is an AST, not a tree-sitter CST. A CST has roughly 1.5–2× more nodes.
- Sub-millisecond times vary about 2× between runs on a busy machine.
- Postings were built on the CPU.

All times are GPU ms. "ovf" counts roots that overflowed the stack, and brackets give the time to build the `anc_P` column.

| Query | Roots | Stack, cap 64 | Interval scan | Posting / owner | SIMD scan [+anc] | Frontier waves (70 dispatches) | Persistent queue |
|---|---|---|---|---|---|---|---|
| Q1 fn has Yield, stopBy scope (EXISTS) | 72k | 3.15 (135 ovf) | 0.88 | 0.021 | 0.15 [+0.21] | 2.58 | 5.1 |
| Q2 fn has Return, stopBy scope (EXISTS) | 72k | 1.34 (127 ovf) | 0.85 | 0.020 | 0.11 [+0.14] | 1.61 | 11.0 |
| Q3 count Call in fn, stopBy end | 72k | 1.86 (142 ovf) | 2.05 | 0.050 | 0.45 [+0.63] | 2.33 | 5.2 |
| Q4 Module has Lambda (huge roots) | 2,049 | 5.38 (467 ovf) | 10.56 | 0.005 | 0.86 [+0.14] | 2.55 | 5.0 |
| Q5 each Call has Lambda (nested roots) | 421k | 0.26 (25 ovf) | 0.11 | 0.026 | 0.20 [+0.16] | 0.90 | 5.7 |
| Q6 each Call inside Try, stopBy scope | 421k | — | parent walk 0.12 | — | — | — | — |

- **Correctness:** every formulation had 0 mismatches, except the stack variant on roots that overflowed.
- **For comparison:** a single-threaded CPU interval scan took 11–190 ms on the same queries (noisy).

## Recommendations

1. **Adopt stratified range execution as the Metal model** within decision 0004.
   - IR instruction words remain for local tests and bounded paths (Tempo classes 1–3).
   - Relations, `EXISTS` and nesting lower to set operators, not to `FORK` path enumeration.
2. **Decision 0003 must require fixed-width `size` and `parent` per document,** plus kind postings from the index.
3. **Add the verifier rules and capability flags,** with `Unknown` plus a CPU/JIT rerun as the only fallback.
4. **Replace the 600-dispatch and 2 GB guards with bounds reported at plan time.**
