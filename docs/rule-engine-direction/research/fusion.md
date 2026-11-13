# Fusing co-executed transducers

> **Superseded (2026-10-25).** This report predates `metal-enqueue.md` and the no-fallback rule (spec §8). Statements below that route anything to a CPU/JIT rerun are replaced: monotone recursion runs on Metal as GPU-scheduled waves, constructs that cannot be stratified are rejected by the verifier, and hard device limits yield `Unknown(resource)`, which is counted and gated.

Benchmark sources and raw output are in `../experiments/fusion/`. These are CPU measurements only. The Metal claims below are structural (task counts, frontier boundedness, pass shape), not measured.

## Setup

- **Interpreter.** A minimal copy of Tempo's transducer core loop (`policy_dsl/transducer.rs` opcodes at `:184-216`, `Path` at `:813`, tape interpreter at `:2600-2800`): one 32-bit word per instruction, with one path forked per navigation result.
  - It adds `ANCESTOR`, `EXISTS`/`NOT_EXISTS`/`ACCEPT`, `DESCEND_KIND`, set mode with label bitsets and per-pc reachable-label masks, lazy enumeration (`LAZY=1`) and the peephole (`PEEP=1`).
  - It omits captures, value programs and repeats.
- **Tape.** A regular table (kind, field, named, parent, next sibling, first child, subtree end) built from tree-sitter-python 0.25 parses of the Python 3.14 stdlib: 1,847 files, 8.54M nodes.
- **Programs.** Templates harvested from corpus statistics: child-field, has, inside and sibling templates over (parent, field, child) triples.
- **Correctness.** Every variant produces the same output set (asserted).
- **Noise.** Each figure is the minimum of 3–5 runs on a shared machine, with ±20–40% variation at small absolute times. Differences under 1.3× are noise.

## Results

| Fusion | Result |
|---|---|
| **Shared relation-mask passes** (one reverse-preorder pass gives each node a bitset of target kinds in its subtree; one forward pass does the same for ancestors; each `has`/`inside` is then a bit test) | 210 programs: 348–442 → 6.4–17.3 ns/node, **25–54×** |
| **Multi-rule tries by root kind** (dense kind→group dispatch, shared prefixes, local labels, per-pc reachable-label masks) | K=512: **2.3–2.5×**, tasks 62.5M → 4.37M (14.3×); K=128: 1.3–1.5×; K≤32: about break-even |
| **Fused dispatch without an index** | As fast as anchored execution with prebuilt postings, whose postings cost 12.4 ns/node to build. Right for per-edit checks |
| **Lazy descendant cursor + `DESCEND KIND` peephole** | Pushed paths 5.26M → 96k (55×); about 2–2.5× faster. `DESCEND` becomes a one-entry cursor, which moves `has` programs out of Metal's dynamic-frontier class |
| **AND/OR/NOT fused on one root** | **1.3–2.0×** against separate programs plus a hash join. Conjunct order matters by up to 1.8×; order by `cost/(1 − selectivity)`. Not a win when the first conjunct barely filters (A AND NOT B at 93% A: 0.84–1.07×) |

Multi-rule detail (lazy + peephole, ns per corpus node, best of two runs):

| K | Separate anchored | Fused dispatch | Fused trie | Tasks: separate → fused |
|---|---|---|---|---|
| 8 | 2.36 | 2.74 | 2.54 | 0.95M → 0.50M |
| 32 | 19.5–29.7 | 23.7–27.0 | 21.4–33.5 | 5.26M → 2.62M |
| 128 | 71.9–92.6 | 67.2–81.0 | 51.4–61.6 | 17.3M → 3.75M |
| 512 | 363–496 | 265–268 | 160–204 | 62.5M → 4.37M |

At K=512, code shrank only 1.25× (2,194 → 1,750 words). The gain comes from shared navigation: one child enumeration serves 25 child-field programs.

## Fusion forms

- **OR / `any`:** `KIND r; FORK Lb; EXISTS A; EMIT l; Lb: EXISTS B; EMIT l`. In set mode a task stops at its first emit. Shared prefixes run once.
- **AND / `all`:** `EXISTS A; EXISTS B; EMIT`, in sequence with early exit. A product construction only helps when both conjuncts navigate the same axis, and tries already cover that. A capture slot is exported only when a later equality reads it.
- **NOT:** `EXISTS A; NOT_EXISTS B; EMIT`. The same primitive gives `stopBy: <rule>`.
- **Relations.** Three lowerings, in increasing order of fusion:
  1. an existence barrier with a lazy range cursor;
  2. `DESCEND_KIND k`, a contiguous scan of the `kind` column over `[node+1, end)`;
  3. batch mask passes:
     - subtree pass: `sub[parent] |= sub[child] | bit(R at child)`, in reverse preorder;
     - ancestor pass: `anc[i] = anc[parent] | bit(R at parent)`;
     - a rule nested d levels deep needs d passes;
     - `stopBy: neighbor` is one level, `stopBy: end` is the closure, and `stopBy: <rule>` gates propagation on NOT stop;
     - use masks when `Σ_roots(subtree scan) > nodes·c_pass/targets_per_pass`; otherwise use lowering 2.
- **Capture equality:** within one match, `ASSERT TEXT_EQUAL` over `CAPTURE_ZIP`. Between two independent matches, keep it in SQL as a hash join over narrow `t_export` relations, unless both sides are tiny.
- **Counts:** `COUNT sub axis cmp n`, with an early stop. Use popcounts of child masks, or per-target prefix counts in preorder, `count[end] − count[node+1]`.
- **Utilities:**
  - small ones are inlined by digest (compile-time dedup took 974k expansions to 316k);
  - large shared ones use `CALL`/`RETURN` with a depth bound;
  - a utility used as a relation target becomes a mask target;
  - recursive ones are CPU/JIT only, unless they can be rewritten as a mask fixpoint.

## Cost model

All inputs live in SQL and are cached with the plan:

- **Root counts** `N_k` from `fts5vocab`.
- **Fan-out and subtree size** per kind, from the edge and size aggregates.
- **Selectivity:** defaults from kind co-occurrence, refined by `plan_feedback(program_digest, s, cost, at)`.
- **Cost per conjunct:** navigation steps × fan-out × measured ns per step.

Decisions:

- **AND order:** ascending `c/(1 − s)`.
- **Fuse group G** when separate cost exceeds fused cost:
  - separate: `N_k·(m·t_task + Σc_i) + m·D`;
  - fused: `N_k·(t_task + c_prefix + Σ c_suffix·reach) + D + δ_div·branches`;
  - in practice: fuse when 16 or more programs share a root kind, or when 2 or more programs share a relation target.

Each plan records the statistic values behind each decision and is recompiled only when a decision would flip.

## Planner

- **Grouping:** group only programs with the same candidate-scope digest and grammar (1.45× for identical scope against 0.86× for a union).
- **Cache key:** (sorted member digests, fusion-plan version, IR version, grammar, statistic buckets used).
- **Demultiplexing:**
  - `t_match(program_group, label, doc, root)` joined with the label table `label → (rule, fragment)`;
  - u128/u256 bitsets above 64 labels, or deduplication left to consolidation;
  - an Unknown group is split and re-run.
- **Per edit:** fused dispatch without an index, plus masks, for the one file.

## What the findings contract makes legal

Each of the following is allowed only because findings are a set (decision 0001). Under exact ordered or witness semantics, every one is forbidden:

- first emit per label, then stop;
- `EXISTS` barriers;
- reordering conjuncts;
- OR as a short-circuit union;
- trie merging;
- label pruning;
- lazy enumeration;
- mask passes, which produce no witness paths at all.

A rule that really is ordered becomes `A AND NOT_EXISTS B`, which is itself a fusion.

## Recommended IR additions, in order

1. Set mode: first emit per label, a label bitset, and per-pc reachable-label masks.
2. `EXISTS`, `NOT_EXISTS`, `ACCEPT`, with verifier-bounded nesting.
3. Lazy range enumeration for `DESCEND`/`FOLLOWING`/`ANCESTOR`, plus `*_KIND` superinstructions. These need fixed-stride `kind` and `end` columns.
4. A program-group header: a dense dispatch table from root kind to entry pc, plus the label table.
5. A mask stage: batch-level bottom-up and top-down passes producing per-node bit columns, read through `MASK t`.
6. `COUNT sub axis cmp n` with an early stop.
7. `CALL`/`RETURN` with a depth bound.
8. Per-label work bounds, and splitting a group that goes Unknown.

## Relationship to the Metal recursion design

The mask passes and the stratified range execution in `metal-recursion.md` are the same idea. A relation's inner rule is materialized as a per-node set (a bitset column or a posting list) by a pass over the regular tape, and the outer rule tests membership. Fusion supplies the multi-rule form: one pass carries the target bits of many rules. Metal recursion supplies the bounds and the dispatch shape.
