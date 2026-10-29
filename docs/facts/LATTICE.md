# Lattice facts: design invariants (scratchpad)

Status: **proposal under exploration, 2026-10-29.** Not binding; `INVARIANTS.md` I1–I10 still govern everything outside declared lattice facts. Origin: Jeremy's claim that replacing the static lattice with declared, memoised recursion "buys out a lot of complexity", the user's direction that the fixpoints run as a parallel matrix across rules, and two constraints from the user: execution must have a **very regular shape** to be fast, and on Apple silicon the data **should not move** (unified memory). Numbers are copied from a cited run or marked `[unverified]`.

## Thesis

Rules are Datalog over declared facts. Recursion is a declared monotone fixpoint over a lattice from a closed catalogue; the engine owns its evaluation, its parallelism and its maintenance. Every fixpoint round is the same regular operation: a sparse matrix (an edge relation in compressed layout) times a dense frontier, over the lattice's semiring, on columns that every worker reads in place.

```
rule  → reads declared facts (DECLARED-FACTS.md)
fact  → non-recursive: plan as today (I5, I7)
      → recursive: { seeds, edges, lattice } → stratum
stratum → rounds: frontier' = (edges ⊗ frontier) ⊕ values, until no value changes
        → parallel over contents | over independent facts | over frontier partitions
edit  → insert: continue the rounds from the new seeds/edges
      → delete: the lattice's own maintenance algorithm, over the affected frontier
```

## Why this could remove complexity (what is claimed)

A monotone fixpoint over a lattice reaches one least fixpoint whatever the evaluation order, the partition and the thread count (joins are commutative, associative, idempotent). If every recursion is of that kind, the engine needs no ordering between rules inside a stratum, no per-rule delta rules, no locks on derived state, and no order-dependent refusals inside the recursion. The cost moves to a small, fixed set of engine-owned algorithms: one evaluator, one maintenance algorithm per lattice.

## Invariants

| Id | Invariant | Forbids | Enforced by |
|---|---|---|---|
| L1 | **Recursion is declared, not written.** A recursive relation is a `facts` entry `{ seeds, edges, lattice }`: seeds and edges are non-recursive plans over facts; the lattice names how a value is kept and combined. Rules read the fact by name. A legacy `WITH RECURSIVE` runs on the existing `Fix` (I1 semantics) unless a registered rewrite proves it equal to a declared lattice fact (I4). | Recursion spelled out in rule text for new rules; the engine recognising a CTE's shape and swapping in a lattice without a registered, side-conditioned rewrite | compiler: `facts` with `lattice` is the only recursive construct new packs may use; rewrite registry |
| L2 | **The lattice catalogue is closed and engine-owned.** Each entry states: carrier, join, bottom, the ascending-chain bound (finite height, or bounded by input size), the semiring used by a round, its maintenance algorithm, a differential test against the reference and a mutant. Starting catalogue: `set` (∨, ∧ over booleans), `min_level` (min, + over ℕ ∪ {∞}), `best_witness(key)` (lexicographic min over a key proven total by I2). Integer carriers only, so results do not depend on reduction order. | User-defined joins; float carriers; a lattice whose height the engine cannot bound; "best" without a total key | catalogue in code; per-entry test + mutant; compile-time refusal of anything else |
| L3 | **Monotone inside a stratum, ordered between strata.** A step reads its own stratum only through monotone operators (join, union, projection, the lattice's ⊗). Negation, aggregation, windows and `LIMIT` read only completed strata. | Negation or aggregation over the relation being computed; non-stratifiable programs | dependency analysis in the compiler; refusal by construct (I9) |
| L4 | **Results are independent of schedule.** The result of a stratum is a function of its inputs alone: identical across thread counts, partitions, backends (CPU, GPU) and round orders. Thread count, partition and backend are never part of a memo key. | Results or tie-breaks that depend on which worker got there first; per-thread memo keys | gate check: every lattice fact evaluated at 1 and N threads and on each backend, bit-identical |
| L5 | **Regular execution shape.** A round is one bulk operation: edges in CSR (u32 offsets, u32 targets, both sorted), the frontier as a bitset or a sorted u32 vector, lattice values in a dense array indexed by a u32 id, the step as sparse-matrix × frontier over the semiring. No allocation per row, no hash map and no pointer chasing in the inner loop, no per-node dynamic dispatch; work units are equal-sized ranges of frontier or of edges, not variable-sized per-key tasks. | Per-key memo re-runs as the evaluator of a fixpoint (they may stay as bookkeeping around it); `HashMap` probes per derivation; `Vec` per node; work stealing over tasks of unbounded size | plan audit (no hash or alloc operator inside a lattice round); counts per round (L8); review |
| L6 | **Data does not move.** Facts live once, in their execution layout, in memory every worker reads in place: columns mapped from the content store, shared read-only by all CPU threads and, when used, by the GPU through shared buffers (Apple unified memory; page-aligned no-copy buffers). Workers own only their output slices, merged by the lattice join. No serialisation between stages, no per-worker or per-device copy of a shared fact, no host↔device transfer. | Copying facts into a worker or device; converting columns between stages; a second resident copy of a shared relation | memory accounting counts mapped pages once and reports private bytes per worker; a copy-count counter in the physical counts (`LOOP.md`) must read 0 for shared facts |
| L7 | **Parallelism is the engine's, on three axes only:** across contents (per-content facts, I7), across facts of one stratum that do not read each other (the fact DAG), and within a round (frontier or edge ranges, merged by join). It is never across rules that share a fused pass over one content: fusion (one pass per file over all selections) wins over splitting rules on the same content. | Rule-level threads over the same content's facts; parallelism that changes a result (L4) | scheduler is the only component that creates work units; fused passes are single units |
| L8 | **Cost is visible per round.** Each lattice fact reports rounds, frontier width per round, derivations, edges scanned, and bytes moved, as algebraic counts (`LOOP.md`). Rounds bound the critical path; frontier width bounds the parallelism available in a round. A fact whose rounds exceed its declared bound, or whose work exceeds O(edges × height), is a named refusal. | Wall-clock-only claims of parallel speedup; unbounded rounds | `EXPLAIN ANALYZE` counts; the count-regression check |
| L9 | **Maintenance is per lattice, not per rule.** Insert: continue the rounds from the new seeds and edges (monotone, so exact). Delete: the lattice's algorithm over the affected frontier, still in the L5 shape: `set` by support counts or over-delete-and-rederive restricted to the affected ids; `min_level` by levelled re-check (a node keeps its value while a predecessor at level − 1 survives; Y2's reached set); `best_witness` by re-check of the nodes whose witness was removed. Incremental ≡ from scratch on the replay. | Hand-written delta rules per fact; recomputing a whole stratum on an edit when the lattice has an algorithm | replay check after every event (as Y2/Y3); per-lattice mutants |
| L10 | **Layered state.** A lattice fact's values follow the membership layering: base, cluster overlay, worktree delta (Y1). Per-worktree state is proportional to divergence; a new worktree costs nothing. | A full copy of the values per worktree | memory per worktree against divergence, as B4 |

## How this amends I1–I10

- **I1** (SQLite semantics): unchanged for legacy recursion. A declared lattice fact has lattice semantics, not `WITH RECURSIVE`'s; it is a new construct (I1's last bullet: new constructs enter through the IR), so SQLite is not its oracle. Its oracle is the reference evaluator's naive fixpoint.
- **I5**: adds one physical operator, the semiring round over CSR; the existing `Op::Fixpoint { seminaive }` (`spikes/facts-a/src/physplan.rs`) remains for legacy recursion.
- **I8**: for lattice facts, incrementality is per lattice (L9), not generated per plan. This resolves the I8 vs Y3 inconsistency for recursive relations; non-recursive membership plans keep I8 as written.
- **I9**: a user lattice is refused by name until the catalogue admits it (L2).

## Hypotheses and what would kill them

| Id | Hypothesis | Measured by | Kills it |
|---|---|---|---|
| H1 | The recursion in the 60 rules fits the catalogue (`set`, `min_level`, `best_witness`) | E0 census of the 68 `WITH RECURSIVE` in the 8 recursive rules | > 20% of the recursive CTEs need a lattice outside the catalogue, or are not monotone |
| H2 | The recursive work is a large share of the heavy rules' cost | E0, algebraic counts per stage on batch 6 | recursive stages < 20% of the 8 rules' counted work: parallel fixpoints are then not the lever |
| H3 | Rounds are few and frontiers wide enough to parallelise | E0, rounds and frontier width per round on the corpus | median frontier width < 10K ids, or rounds > 100 on the heavy facts: per-round parallelism cannot pay for synchronisation |
| H4 | The regular shape makes one core much faster than the current evaluators on the same fixpoint | E1, single thread, against Y2's reached set and `Op::Fixpoint` on the same input | < 2× on one core |
| H5 | The regular shape scales across cores | E1 at 1, 2, 4, 6, 12, 18 threads (this machine: 18 cores, 6 + 12, 128 GB unified) | < 4× at 6 threads, or peak memory grows with threads beyond output slices |
| H6 | The GPU pays on wide frontiers with zero copies | E2, only if H3 holds | no win over the CPU at the measured frontier widths, or any copy of a shared fact |
| H7 | Maintenance stays in the regular shape and in Y2's latency | E3, replay | p95 per edit above Y2's 1.5 ms for dead code, or incremental ≢ from scratch |

## Experiments

- **E0 census (no engine change).** For each of the 68 recursive CTEs: what it recurses over (syntax tree, reference graph, call graph, strings), set or bag, what value it carries, which catalogue lattice it is, whether it is monotone, and whether it is per-content or membership. From the existing counts on batch 6: derivations, rounds and frontier width per round, and the recursive stages' share of the rule's work. Answers H1–H3.
- **E1 regular fixpoint on the CPU.** A semiring-round operator (`set`, `min_level`) over CSR on mapped columns; dead-code reachability over Y1's reference graph (cold, oracle tree) as the first input, then the heaviest recursive stage E0 finds. Exact against Y2's reached set and the reference; counts and wall at 1–18 threads; private bytes per worker. Answers H4, H5.
- **E2 Metal backend (after E0).** The same round as a compute kernel over shared no-copy buffers; bit-identical to E1. Answers H6.
- **E3 maintenance (after E1).** L9's `min_level` and `set` deletion in the L5 shape on the 1,111-event replay. Answers H7.

## Open questions

1. **Arbitrary lattices (Jeremy's position).** L2 closes the catalogue. The case for openness is generality and authors not blocked on the engine; the case against is that a user join that is not monotone or has unbounded height breaks termination and exactness, and the engine cannot check it. To settle with him, with E0's census as the evidence.
2. Type inference (`type join`) is not in the catalogue: it needs a pack-declared finite lattice. E0 says whether any of the 60 rules needs it.
3. Whether per-key memoised maintenance (Y1/Y3) and the bulk shape coexist: Y1's keyed queries for the non-recursive graph, L5 rounds for the fixpoints; or Y1's graph itself rebuilt as CSR with bulk deltas.
4. Fusion and L7: whether a fused per-file pass and per-round parallelism ever compete for the same content.
