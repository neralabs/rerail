# 0097: What a relation keeps: recompute in the reader, restrict by demand, or store, decided per relation

Status: draft (2026-11-09). A perspective, built only as far as 0096's projection (`count only`).

## Intro

**The question (the user).** "Some of the relations should maybe not be materialized. Something which creates a
massive expansion in an intermediate join before narrowing down should maybe be recomputed in the intermediate join
and not serialized. This could be done by analysis or explicitly in the program."

**The answer in short.**

- **It is the norm in this engine's cost, not an edge case.**
- **Mostly by analysis.** The relations that expand and then narrow are mostly ones the converter and the engine
  make (`rr_agg*`, `fl*`, `demand_*`), which no author would annotate. So the planner should decide, per derived
  relation, whether to store it, recompute it inside its readers, or restrict it by demand.
- **An annotation is an override:** Soufflé's `inline` / `no_inline`, `magic` / `no_magic`.

**How to read this record.** First what the Python pack shows. Then the options and when each wins, the decision
proposed, and what is built.

## What the Python pack shows

These are estimates from the profile taken after 0095's fixes (12.45 s of placed CPU, 139.9 M units of join work)
and the relational lab's 144 captures. No inlining experiment was run.

- **Native strata write 58.9 M rows.**
- **208 relations, 28.0 M rows, are written large and read small.** That is 47.6% of the rows and 45% of the join
  work, about 6 s. Each of them has at least 10 k rows, is read with some columns bound, and its readers keep at most
  a tenth.
  - Their readers probe them 1.44 M times and keep 273 k rows.
  - 154 keep under 1%. 150 have a single reader.
- **How their readers read them:**

  | Read | Relations | Rows |
  |---|---:|---:|
  | only inside `count {…} = 0` or `min {…}` (the converter's `rr_agg*`) | 38 | 13.0 M |
  | positive lookups | 95 | 8.7 M |
  | file-local pieces (`fl*`) | 36 | 3.0 M |
  | demand's own relations (`demand_*`) | 17 | 2.2 M |
  | negation | 16 | 0.8 M |

- **The largest cases:**
  - **`rr_agg77`**: 3.9 M rows, and as many again in its annotation copy. It joins `rr_inside_ks11` with
    `bound_in`, and its one reader asks `count {rr_agg77(_, s, x)} = 0` of 55,314 pairs and keeps 405. In a capture
    its inputs hold 63 k and 2.6 k rows, and their join 617 k. Recomputed per pair, it costs at most about 8 probes.
  - **`module_of`**: 954 k rows of node, document, module. Its seven readers bind the node and keep 17.5 k.
  - **`fl4683_p207_outer`**: 777 k rows, probed 2,548 times, keeping 647.
  - **Nearly all waste:** `fl5245_p514_effect` (363 k rows, probed 7 times, keeps none) and `p358_final_alias`
    (232 k rows of a cross product, negated by 3).
  - **Demand making the blow-up:** `demand_tp_name` holds 338 k rows to answer 155.
- **Inside one rule:** 50 plans pass a step of 100 k rows or more that is at least 10× their output, 16% of the
  work. The worst are an equality applied after a cross product: `fl4317_typing_root` goes from 14 k rows to 2.86 M
  to 603. That is a planner bug, not a storage question.

**Why demand (0048) misses them.**

- It drops a relation read inside an aggregate, and `count {…} = 0` is how the converter writes "no such row".
- It drops a relation whose rule has a variable of unknown type.
- It is off in sessions. A relation computed for some keys only can miss a key asked later (O125).
- `inline` on a `.decl` is refused ("not yet").

## The options

| Option | Wins when | Costs |
|---|---|---|
| **Store it** (today) | Many readers, or many probes per key; recursive; read unbound; an output | Rows written, indexed, kept per chunk and maintained across edits |
| **Recompute in the reader** (inline) | Not recursive; every reader binds a key from which the relation's body is cheap; probes × that cost ≪ its size. Existence reads gain most: they stop at the first match. | The reader is copied once per rule of the relation. Work repeats if many probes hit one key. A count, sum or string join needs the distinct rows, so only existence aggregates inline freely. |
| **Restrict by demand** (magic sets, 0048) | Many probes share few keys | Extra strata. A demand relation can itself be wide. Unsound in a session unless demand is maintained too. |
| **Project it** (`count only`, 0096 §1) | Readers ignore some columns | None: the rows are fewer and the same to every reader |
| **Store it factorized** | It must cross the chunk boundary, or is read only by key or to count: `rr_agg77`'s two inputs (66 k rows) instead of their join (617 k) | A second layout for readers and kernels |
| **Better join order** | The blow-up is inside one rule | Nothing for a relation whose own output is large; only the reader's binding narrows that |

Two consequences beyond CPU:

- **Maintenance across edits (0090, 0092).** A kept relation is paid for per row on every run, and a wide one widens
  over-deletion. An inlined relation is maintained through its inputs, which are maintained anyway. So inlining
  shrinks the kept state and stays exact.
- **Device waves (0095 §5, 0096).** Inlining is kernel fusion: no intermediate to size, keep resident or spill.
  Storing pays on the device only when several waves read the rows.

Memory matters as much as CPU: the pack's peak is 1.39 GB against the 1 GB budget, and the chunk size is set by
memory.

## Decision (proposed)

1. **The planner decides per derived relation, after `simplify` and before demand:** store, inline, or demand. Its
   inputs are the relation's size and the work to build it, each reader's probes into it (the plan's rows into the
   step), its body's cost per probe given the reader's bound columns (to the first match for an existence read), the
   distinct probe keys, and a per-row price to store, index and maintain:

   ```text
   inline       Σ probes × fan-out < α (work + m·|R|), every reader binds a key the body reaches from,
                at most K rules, and every aggregate read is an existence test
   demand       Σ keys × fan-out ≪ work, and probes ≫ keys (corpus runs, until demand is maintained)
   store        otherwise; always when recursive, an output, read unbound, or read by readers whose probes
                sum past |R|; factorized when it crosses the chunk boundary
   ```

   The estimates are the plan's on the first run, and the observed step counts afterwards (`native.tsv` has both).
   The profile names every choice and why, as demand's do.
2. **An existence count is a negation.** `count {R(…)} = 0` is read as `!R(…)` by demand and by inlining, as the
   projection already does (0096). That alone brings in the 13 M rows read only inside such counts.
3. **The program may override.** `.decl r(…) inline` and `no_inline`, `magic` and `no_magic`, as Soufflé spells
   them. The converter writes `inline` on the helpers it makes when the analysis would, so the choice is visible in
   the program.
4. **Separately:** an equality after a cross product inside one rule is a planner fix. Worst-case-optimal joins are
   for cyclic bodies only.

## Built

- **Projection (`count only`), 0096 §1:** the cheapest of the options, and the first.

Next is a lab experiment: inline `rr_agg77`, `module_of` and `fl4683_p207_outer` into their readers' captured specs,
and compare CPU and rows. Those numbers set `α`, `m` and `K`.
