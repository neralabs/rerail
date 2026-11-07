# Transducer fusion: normative specification

Status: draft for review. This document is normative for the set-mode transducer IR, its fusion transformations, and the operations fusion needs.

Inputs:
- `research/fusion.md` and `experiments/fusion/`, the CPU measurements;
- `research/metal-recursion.md`, the stratified range execution;
- `research/transducer-ir.md`;
- `spec.md` §6–§9 and §11;
- Tempo's IR, `tempo/native/src/policy_dsl/transducer.rs` at `e95613625`:
  - opcodes at `:184-208`;
  - word layout at `:1111`;
  - the verifier at `:1628-1700`;
  - the interpreter at `:2099-2300`.

Constraints taken as given:

- **Findings are a set** (decision 0001, `findings-contract.md`). Every transformation here relies on it.
- **No CPU/JIT fallback from Metal.** Every operation in this document has a Metal implementation. Unbounded work runs on Metal through GPU-driven continuation (device-side enqueue; the companion design, C2). Resource exhaustion yields `t_outcome = Unknown(resource)` for the affected (label, document). It is reported and never rerouted to another backend. The planner may resubmit on Metal with a smaller batch or a split group.
- **The CPU interpreter is the reference semantics and the test oracle,** not an execution fallback. The CPU JIT is a first-class backend for CPU-only deployments. Both execute exactly the same admitted programs as Metal.

---

## 1–3. Machine model, operations and group tables

Moved to `spec-transducer.md` §1–§3 with numbering unchanged. References below to §1, §2 and §3 mean that document.

---

## 4. Fusion algorithm

The input is the set of compiled per-rule programs that the planner assigns to one batch, each with its candidate-scope digest. The output is program groups with their set stages, plus a demultiplexing table.

### 4.1 Normalization

Each program `P` is rewritten into canonical form:
1. Lower `AstMatcher` features to IR, per the frontend.
2. Apply peepholes: `CHILD; KIND k` becomes `CHILD_KIND k`, and likewise for the other `*_KIND` forms.
3. Reorder adjacent pure tests on the same node into canonical order: `KIND`, `FIELD`, `FLAGS`, `NTH`, `SIDE_TEST`, `SIZE`, `TEXT`, `ASSERT`. Tests are side-effect free, so reordering preserves the path set.
4. Number captures canonically, by first occurrence.
5. Remove unreachable code.
6. Compute `digest(P)`.

Programs with equal digests are deduplicated: one program, several labels.

### 4.2 Grouping

```
groups := partition(programs, key = (scope_digest, grammar, backend_profile))
for each group g:
    split g by label budget (≤ W labels per group on Metal)        -- §6.2
    split g by code size (≤ 65,536 words after fusion)             -- §6.1
```

Only programs with an identical candidate scope share a group. This is the measured batching lesson: 1.45× for identical scope against 0.86× for a union (`README.md` §5).

### 4.3 Prefix tries by root kind

```
for each root kind k:
    members := programs whose first instruction is KIND k (or *_KIND k at the root)
    trie := new
    for P in members (sorted by digest):
        insert P's instruction sequence into trie, sharing the longest common prefix
        (equality = identical word and identical table entry content)
    emit trie: shared prefix once; at each divergence point with children c1..cj:
        FORK c2; ... FORK cj; c1 ...       -- j−1 forks
    each leaf ends in EMIT label(P)
dispatch[k] := entry pc of trie(k)
```

Divergence inside a sub-program region is not shared: regions are copied per member unless they are identical by digest, in which case the region is shared.

### 4.4 Boolean fusion within one rule

For a rule fragment `all/any/not` over sub-rules on the same root:

```
OR(A1..Aj):  FORK L2; <A1 as inline> EMIT ℓ; L2: FORK L3; <A2> EMIT ℓ; ...
             -- the first EMIT ℓ fires the label; reachable-label pruning drops the rest
AND(A1..Aj): order Ai by cost(Ai) / (1 − sel(Ai)) ascending   (§4.8)
             for each Ai: if Ai binds captures read later or exported → inline Ai
                          else → EXISTS sub(Ai)                (barrier)
             EMIT ℓ
NOT(A):      NOT_EXISTS sub(A)
```

Inline versus barrier is a legality decision (§5.2), not only a cost decision.

### 4.5 Utilities

```
for each utility u with use count n_u and size s_u (words):
    if u in recursive group:              lower group to FIX set; call sites → IN_SET  (§2.6)
    elif u used as a relation target:     LEAF/HAS/INSIDE set of u; sites → IN_SET
    elif s_u · n_u ≤ INLINE_BUDGET (=256 words) or n_u ≤ 8: inline at each site
    else:                                 keep as CALL region (shared by digest)
```

The thresholds come from `fusion.md` §2.8 and are calibrated by `backend_costs` (§4.8).

### 4.6 Relation lowering

For each relation `has/inside/precedes/follows` with inner rule `I` at root set `Roots`:

```
cost_cursor = Σ_{r ∈ Roots} E[|axis(r)|] · c_step
cost_kind   = Σ_{r ∈ Roots} E[|axis(r)|] · c_scan          (only if I's first test is KIND k)
cost_set    = n · c_pass / T + |Roots| · c_test + cost(LEAF(I))
choose min; set stage is mandatory if I contains an unbounded axis inside a sub-program at
nesting > 2, or if I is a recursive utility.
```

`T` is the number of targets packed per pass: the count of other relation sets of the same pass type in the group, up to the word width. Packing therefore makes the set stage cheaper the more rules share a pass. That is the measured 25–54× at 72 targets.

### 4.7 Stratification and labels

1. Collect every set definition from §4.5 and §4.6. Deduplicate by (type, parameters, inner-set digests).
2. Compute strata (§2.6). Pack passes per stratum by (type, depth, stop, field).
3. Assign a dense label id to each (rule, fragment), and compute `R(pc)` by backward dataflow over the CFG.

### 4.8 Cost model

Every parameter, with its source:

| Symbol | Meaning | Source |
|---|---|---|
| `n` | nodes in the batch | `workspace_docs` ⋈ `search_index_sets.node_count` (`spec-indexes.md` §2.4, §2.9) |
| `N_k` | roots of kind `k` in the batch | `search_kind_vocab.cnt` for `gcode(k)` × (batch docs / store docs), or an exact count from the `instance` join when the batch is a workspace subset (`spec-indexes.md` §2.9) |
| `D_k` | documents containing `k` | `search_kind_vocab.doc` |
| `f_{k→k'}` | mean `k'`-children per `k` node | `edge_freq(k, k', ·) / N_k` (`spec-indexes.md` §2.5) |
| `s_k` | mean subtree size of `k` | **new** aggregate `kind_size(gcode, nodes, sum_size)`, maintained with `edge_freq` (one row per kind, negligible size). It must be added to `spec-indexes.md` §2.5 |
| `sel(P)` | probability that P accepts at a candidate root | `plan_feedback.selectivity` (EWMA) if present, else the prior `min(1, D_{k'}/D_k)` for a relation to kind `k'`, else 0.5 |
| `c_step`, `c_scan`, `c_test`, `c_pass`, `t_task` | ns per navigation step, per contiguous kind-scan node, per bit test, per node per set pass, and per task | **new** table `backend_costs(backend, device, op, ns, measured_at)`, filled by a calibration microbenchmark at install and on driver change |
| `D_disp` | overhead per dispatch (Metal) | `backend_costs` |
| `δ_div` | divergence cost per fork in a SIMD group | `backend_costs` (calibrated as the slowdown of a trie with `j` forks against `j` separate programs at equal work) |

Decisions:
- **AND order:** ascending `cost(A_i) / (1 − sel(A_i))`.
- **Fuse group `g` (a trie):**
  - separate: `Σ_k N_k · (m_k · t_task + Σ_i c_i)`;
  - fused: `Σ_k N_k · (t_task + c_prefix,k + Σ_i c_suffix,i · reach_i) + δ_div · forks_k`;
  - fuse when separate exceeds fused × 1.1.
- **Relation lowering:** §4.6.
- **Set representation:** use a posting list when `bound(S) · 32 < n`, otherwise a bitset. `bound(S)` propagates from `N_k`: for `HAS`/`INSIDE` it is at most the outer kind count, for `AND` the minimum, for `OR` the sum.

`plan_feedback(program_digest, backend, selectivity REAL, ns_per_root REAL, samples INTEGER, updated_at INTEGER)` is updated after every run from the executor's per-label counters (§7.4).

### 4.9 Plan-cache key

```
key = SHA-256( sorted(member canonical digests) ‖ fusion_version ‖ ir_version ‖ grammar_set_digest
             ‖ backend_profile ‖ stats_bucket_vector )
```

`stats_bucket_vector` holds `⌊log2⌋` buckets of every statistic a decision read. The plan stores each decision's interval, and re-validation follows `spec-indexes.md` §2.12. The key is stored in `compiled_plans`, as one fused-group plan per key.

### 4.10 Demultiplexing

- The executor emits `t_match(group, label, doc, root)`, `t_export_<schema>(group, label, doc, root, …)` and `t_outcome(group, label, doc, status, reason)`.
- The SQL layer joins the label table `labels(group, label, rule, fragment)` to produce per-rule relations.
- `t_outcome` is per label, so an `Unknown` caused by one label's budget does not mark other labels `Unknown`, provided the executor charges work per label (§6.9).

---

### 4.11 Priority on the data-layer ruleset (measured)

> **Revised by M2 (`research/m2-m7.md`).** With regex results interned per distinct text (`spec-transducer.md` §5.2), memo columns stopped paying (10–15% slower than plain on ASD), and a **recursive** prefix trie plus cost-ordered conjuncts gave the largest gains (2.95 → 0.26 ns per program-node). A single-level trie that shares only the root kind fails once rules are combined (326M evaluations). Keyed lookup turned M7's quadratic join into a lookup (0.451 s → 3.6 ms); in the engine this is L5's indexed SQL join (measurement M10), since keyed sets were withdrawn from the IR (decision 0005). The order below is therefore: interned text tests and L5 indexed joins; recursive tries with conjunct ordering; hash-consed sub-rules; memo columns only where a shared sub-rule is expensive and not text-only; masks last.

`research/fusion-model.md` ran all 535 data-layer queries in every mode, with identical findings. Implementation order follows the measured gains:

1. **Batch-wide hash-consed rule graph with lazy per-file memo columns** for every rule-graph node referenced by two or more queries or relations. This is the step from §4 that deduplicates set definitions by digest, applied to *all* sub-rules, not only to relation targets. Measured gain: 1.4–3.1×, mostly from regex text scanned (2.59 TB → 3.1 GB). This is required, not an optimization.
2. **Masks** only for range-scanning relations that meet the §4 threshold. Neighbor relations are never masked. Masking every relation measured 0.74–1.5×.
3. **Root-kind dispatch and tries**: 0.9–1.2× here. The largest group is K=166. Implement them after items 1 and 2; they remain useful for larger catalogs.

Memo column cost (fitted): about 94 ns per memo hit including allocation, against 5.9 ns per mask lookup. A memo column is promoted to a mask column when its expected lookups exceed nodes × (mask-cell cost / mask-lookup cost).

## 5. Legality

The common argument: each transformation preserves `Out(G, D, r)` from §1.4 as a set. It may change multiplicity, order, the witness that is found, and work. None of those is observable under decision 0001.

| Transformation | Equivalence argument | Illegal when | Planner action when illegal |
|---|---|---|---|
| Test reordering (§4.1) | tests are pure functions of the node; conjunction is commutative | a test reads a capture made later on the same path (impossible in canonical form, since captures precede the tests that read them) | none |
| Prefix trie (§4.3) | the trie's path set is the disjoint union of the members' path sets; labels keep outputs separate | members disagree on a table entry with the same index (fixed by renumbering tables in the merged group) | renumber |
| First emit per `KEY` label, then stop | `Out` for a `KEY` label needs only existence | the label is `EXPORT ALL`, or `COUNT` | mark `ALL`; no early stop for that label |
| Reachable-label pruning | a dropped path can only emit labels already in `Out` | as above | as above |
| `EXISTS` barrier for an AND conjunct | `∃` over conjunct witnesses distributes when no capture flows between conjuncts | a later conjunct, an export or a capture predicate reads a capture bound inside the conjunct | inline the conjunct (sequential, same path); no barrier |
| OR as fork plus short circuit | union of the branches' output sets | a branch exports different values (an `ALL` export) | keep the fork, disable the short circuit for that label |
| Conjunct reordering | conjunction is commutative under set semantics; work differs | conjuncts share captures and are inlined: a reorder must respect def-before-use of captures | topological order constrained by capture use |
| `NOT_EXISTS` | `¬∃` | the inner sub-run is `Unknown`, so the result is `Unknown` (§2.3), never false | propagate `Unknown` |
| Relation → set stage (`IN_SET`) | `x ∈ H ⟺ ∃y…` by the set definitions in §2.6, which are exactly the relation semantics | the relation's inner rule binds captures used outside the relation (for example a metavariable shared between the outer pattern and the `has` target) | keep the relation inline as cursor or `*_KIND` navigation |
| Utility inline / `CALL` / set target | equal by substitution | a utility used as a set target binds captures read by the caller | `CALL` or inline |
| `COUNT` → `COUNT_SET` | the count is over the same node set | `sub` reads outer captures | keep `COUNT` with a sub-program |
| Lazy enumeration | freedom 3, §1.4 | never | — |
| Ordered semantics ("A unless B first") | expressed as `A ∧ NOT_EXISTS B'`, where B' encodes "before" positionally (`PRECEDING`, or a span comparison) | never as evaluation order | the frontend rewrites to `NOT_EXISTS` |
| Per-label budgets | budget exhaustion makes that label `Unknown`; it never changes which keys are findings | — | — |

**Captures needed above the file** (`t_export`) are the main legality constraint. Any capture listed in an export schema pins the path that binds it. Such a label must be `ALL` unless the schema is a function of the root alone (for example the root's own text), in which case first emit suffices. The frontend sets the `ALL` flag from the export schema.

---

## 6. Limitations and planner responses

### 6.1 Code size and tables

`m ≤ 65,536` words and at most 65,536 entries per table, because operands are u16. A trie over K programs costs about `Σ|P_i| − shared prefixes`; measured at K=512 it was 1,750 words (`fusion.md`). Groups that would exceed the limit are split by root kind, where tries are independent. Sub-program nesting is at most 8 levels and strata at most 16; beyond that the planner promotes inner levels to the set stage, and if strata would exceed 16 it splits the rule across two dispatch rounds, with an intermediate set kept resident.

### 6.2 Labels

Labels are at most 4,096 per group. Executor bitsets are 64, 128 or 256 bits, chosen by the planner per backend. Above the chosen width, the group is split. The alternative of deduplicating labels in consolidation is not used on Metal, because it loses early stop and reachable-label pruning, which are the main fused-trie wins.

### 6.3 Divergence on Metal

Forks in a trie make SIMD lanes diverge. The planner bounds the expected live branches per task through `δ_div` (§4.8), and it sorts tasks within a dispatch by root kind so that a threadgroup executes one trie. When a trie's expected divergence cost exceeds its fusion gain, it is split into sub-tries by second-level kind or field.

### 6.4 Register and threadgroup memory pressure

Per-thread state:
- pc, the node, a cursor per enumeration level (≤ `MAX_SUB_DEPTH + 2`);
- capture slots (≤ 16 live; Tempo's capture arena for more);
- the label bitset;
- counters (≤ 4).

The verifier computes the maximum live state of each program and the planner rejects any group above the backend's register budget. A group over budget is split, or its sub-programs are moved to the set stage, which uses device memory and no per-thread state. Set-stage bitsets live in device memory. Threadgroup memory is used only for prefix-scan tiles.

### 6.5 Nesting depth against passes

A rule nested `d` relations deep needs `d` set-stage strata, or `d` levels of inline cursors. Strata cost `d · n · c_pass / T` regardless of selectivity, and inline cursors cost the selectivity-weighted scans. The cost model chooses per level (§4.6), and the verifier enforces the 16-strata limit.

### 6.6 Capture-equality joins across matches

**Within one document, these joins lower to the IR** (revised after `research/heavy-inputs.md`, which found 29 per-file joins among the relational clauses and all of them expressible this way):
- when one side is reached from the other by navigation, the join is `EXISTS` (or `NOT_EXISTS`) from the reaching side with `TEXT_EQ`/`SIDE_EQ` against the outer capture, since sub-runs read outer captures (§2.3);
- when neither side reaches the other, lowering rule L5 applies (`spec-transducer.md` §6.5): both sides export candidate rows `(doc, key_atom, node)`, SQL semi-joins them on `(doc, key)`, and the result is findings or a `FACT` set fed back to the outer program. A join several rules share is a catalog derived fact (`spec.md` §21);
- the planner chooses between the two by cost (§4.8), because a per-candidate `EXISTS` over a whole function body can go quadratic.

**Across documents** the join stays in SQL as a hash join over two narrow `t_export` relations. Equality within one match is `TEXT_EQ`/`SIDE_EQ`, as before.

### 6.7 Cross-document and cross-file

Never fused. A group runs within one document, with `DOC_GUEST`/`DOC_HOST` limited to the same file. Cross-file composition (`require_each`, `select_when`, reachability, hierarchy) is SQL over `t_export`, `fact_*` and graph vtabs (`spec.md` §10).

### 6.8 Non-monotone recursion

Recursive utilities with negation or counting through the recursive edge, or with recursion in mixed directions, are rejected by the verifier. Properties that need negation through bottom-up recursion ("all branches raise") are native producers or derived facts (for example `py.flow_exit`), read through `SIDE_TEST` (decision 0005). That rejection is a compile error for the rule, not a fallback. The frontend reports the construct. Such rules must be restated, for example with a bounded unrolling depth declared in the rule, or as an engine fact producer.

### 6.9 Regex and value programs in fused tries

`TEXT`/`TEXT_EVAL` value programs (byte DFAs and Lua patterns) are pure tests and fuse like any test. Their cost is data-dependent (text length), so the planner orders them last among a node's tests (§4.1). On Metal, DFA tables shared by several members of a group are deduplicated by digest in the group's table section.

### 6.10 `Unknown` and per-label budgets

Each label has a work budget (`LabelEntry.budget`). The executor charges steps to every label in `R(pc)` of the path that takes them. When one label's budget is exhausted:
- that label is marked `Unknown(resource)` for the task;
- its bit is removed from `R` for pruning purposes;
- the task continues for the other labels.

A whole-task arena exhaustion marks every label not yet fired `Unknown`. The planner then resubmits that (group, document) set on Metal with the group split into smaller groups. It never reroutes to CPU. A document that still exhausts the arena after splitting to single-program groups is reported as `Unknown(resource)`, together with its byte and node counts, as a defect to fix in the executor's memory model.

### 6.11 Per-edit runs

A single file × all rules gives small `n`, so the cost model selects fused dispatch without an index plus set-stage passes for the one document. Plans are cached per `(scope digest = {file}, …)` bucket, using the per-edit plan class. Statistics from that file's own tape (exact kind counts, free during matching) may refine decisions per run without recompiling, because the plan stores alternatives keyed by bucket.

### 6.12 Determinism

`Out` is deterministic (§1.6). Executor-level nondeterminism (task scheduling, atomics in count/scan/scatter) affects only multiplicity and order. Both are removed by the shared consolidation, and by the set comparison in tests.

---

## 7. Test, debug and profile obligations

### 7.1 Differential testing

**Invariants**, on the corpus fixtures and the gate corpora:
- `Out(fused group) = ⋃ Out(member program run alone)`, compared per label, as sets;
- every backend (CPU interpreter, CPU JIT, Metal) produces identical sets;
- set-stage lowering of a relation equals its inline cursor lowering.

**Matrix:** every program in the conformance packs; every data-layer rule; fused groups of size 1, 2, 8, 64 and 512 built from random subsets.

### 7.2 Fusion fuzzer

- It generates random canonical programs from the AstMatcher grammar: tests, relations, booleans, utilities (including recursive groups that pass the progress check), counts and `NTH`.
- It fuses random groups and checks §7.1 on random and real documents.
- It shrinks failures to a minimal (program set, document) pair, saved as a fixture.
- It runs in CI on CPU, and nightly on Metal.

### 7.3 Dumps

`rerail-engine dump-group <plan key | rule ids>` prints:
- the group header: dispatch table by kind name, the label table, and reachable-label masks;
- each trie as an indented tree with kind names, field names and labels;
- the instruction listing with mnemonics, operands resolved through the tables, and sub-program regions;
- set definitions by stratum, with chosen representations and bounds;
- the cost-model decisions, with the statistic values read and their validity intervals.

`--format json` gives the machine-readable form. `--diff <key2>` compares two plans. The dump is generated from the serialized group (§3.5), so it shows exactly what executes.

### 7.4 Profiling and attribution

Executors keep these counters, labelled by backend and device:
- per label: tasks started, paths explored, steps charged, emits, budget exhaustions;
- per set: pass time, set size, representation;
- per dispatch: time, threadgroups, arena bytes used against reserved.

They are exported as `t_profile(group, label | set, metric, value)` and fold into `plan_feedback` (§4.8). A flame-style report attributes batch time to rules through the labels. Shared trie prefixes are apportioned to labels by reach count.

### 7.5 Verifier test suite

Each verifier rule in §2 has a positive and a negative fixture. Malformed groups (jumps out of a region, `EMIT` in a sub-program, cyclic set definitions, non-monotone `FIX`, an unknown kind code, a table index out of range) must be rejected with a specific error code.

---

## 8. Operations beyond the current IR

Moved to `spec-transducer.md` §7.

---

## 9. Changes this implies elsewhere

- **`spec-indexes.md` §2.5:** add `kind_size(gcode, nodes, sum_size)`, the mean subtree size per kind, for the cost model.
- **New tables:** `backend_costs(backend, device, op, ns, measured_at)` and `plan_feedback` (§4.8), both in the content store.
- **`spec.md` §6:** replace "E10 capability flags / route programs Metal cannot run well to CPU/JIT" with "every admitted program runs on every backend; unbounded work uses the set stage and GPU-driven continuation; exhaustion yields `Unknown(resource)`; the planner resubmits split groups on the same backend."
- **`spec.md` §8:** remove the CPU/JIT rerun fallback from the Metal recursion paragraph.
