# 0030: Semantic predicates as labels from fact producers

Status: accepted (2026-11-03, list step 26)

## Context

Open item O24. Decision 0011 lowers a composite matcher to node-label passes plus positive walking
programs. It refuses semantic leaves, which were then the larger share of the refusals:

| Predicate | Clauses |
|---|---|
| `resolves` | 94 |
| `parse` | 13 |
| `arguments` | 13 |
| `resolves.binding` | 11 |
| `formatArgs` | 6 |
| `value` | 6 |
| `binds` | 5 |
| `layout` | 2 |

Before this step, S1 lowered 77 of 286 Python `ast` clauses. 0011's direction is a node label made
by the predicate's fact producer (`transducer::semantic_facts`: "A producer reports only
predicate-local facts. It never executes a nested user rule. SQL joins the rows to separately
lowered relational operands"). The predicate's sub-rules are lowered as labels of their own.

## Prior work consulted

- **ast-index-spike** (`prior-work/ast-index-spike-design-index.md`):
  - **§17, semantic-fact channel.** `t_fact(fid, pre, fkey, fval)` is "sought like vidx" (D S079),
    and a fact-anchored query seeds from it (D S099 L5474-5548). Semantic edges are a separate
    layer keyed by symbols (D S020 L2049-2058). **Agrees:** a fact is per (file, preorder) rows
    beside the syntax layer, and a label reads them like any other node set.
  - **§17, precomputed semantic booleans.** `scope_id`, `in_loop` and the rest were the linter's
    workaround for missing expressiveness (LCS L41-60; LA F2). **Agrees, so the domain is narrow:**
    a fact holds only the predicate's own verdict and its slots' targets, not a column per node.
  - **§19, GPU.** No GPU design in the spike. The adjacent evidence is that per-row machinery, not
    decode, dominates (D S152 L6687-6721). **Agrees:** the device reads fact rows as (node, label)
    constraints in the pattern kernel's pass, with no host loop per node.
- **t11** (`notes/t11-streams-since.md`):
  - **Agrees:** a native producer "implements a generic relation operator. It does not encode a
    data-layer rule" (`:55-59`).
  - **Agrees:** "No new rule interpreter or parallel implementation of rule semantics" (WP3). The
    producer is W's own predicate code, not a copy.
- **Lessons ledger:**
  - #11, narrow per-family tables: **agrees.** A fact's rows are (node, targets…) only.
  - #13, demand-driven extraction: **agrees.** A producer starts only at its predicate's kinds.
  - #16, fix candidate sets before fusing: **agrees.** Candidates come from each predicate's own
    kind sets (`candidate_kinds`).
  - #17, no per-row host callback in the SQL phase: **agrees.** The rows are made before any label
    pass, once per tape, not called back per node.
  - #19, fragments, not an interpreter: **agrees.** The slots are ordinary labels, and the fact
    joins them by node.
- **P decision 0005** (named derived facts):
  - **Agrees:** a native producer is reviewed engine code, and identity is by content digest.
  - **Departs:** these facts are not named in a catalog. They are the clause's own predicate, keyed
    by the query's content (decision 4 below). A catalog name is earned when ≥ 2 rules share a fact
    (0005 §3).

## Decision

1. **A semantic leaf is a fact label** (`matcher::lower::Fact`, `MatchTree::Fact`,
   `compile_matcher::Label::Fact`):
   - The producer is W's predicate.
   - Each sub-rule tested on one target (a binding's site, value or annotation) is replaced by a
     capture, `FACT_SLOT{i}`.
   - It never runs a user rule.
   - A node holds where the producer matches it and each slot's target has the label of the
     slot's sub-rule, which is lowered as a label of its own.
2. **Rows before passes** (`fact_rows`):
   - Per tape, the producer runs at its kinds' nodes under one file evaluation (`FileScopeSlot`),
     so the scope table is built once per file.
   - The rows are (node, slot targets). They are computed in parallel over tapes and timed as
     `labels.facts`.
   - On the CPU a label is a bit per row whose targets are in their slots' labels.
   - On Metal the rows are a `Pass::Fact` in the pattern kernel (`patterns::FACT`). Its
     constraints are (target, label) pairs, settled from scratch as pattern passes are.
3. **Unknown verdicts:**
   - An unknown verdict is no row. That is exact in a two-valued clause.
   - In a three-valued clause, a fact that may be unknown is refused under `not` ("unknown fact
     under not").
   - `resolves` outside flow languages is two-valued.
4. **Identity by content:**
   - A fact's key is the query's content digest, its dialect and the leaf's ordinal in the lowering
     (`GrammarMatcher::source`, `Fact::key`). The digest covers the `ast` with options substituted,
     plus the shared utilities it reaches.
   - Equal queries lower to equal plans. The wave's identity, which the plan cache binds, no longer
     varies between runs.
5. **Refused for now** (O36):
   - sub-rules tested on many targets: occurrence, `redefines`, formatArgs' unused or missing;
   - predicates that bind;
   - `parse`, `host`, `embedded`, `calls` and `capture`;
   - file path, since a label's view has no host path.
6. **Metal capacity:**
   - **Budget miss.** A label run whose interpreter workspace misses Metal's buffer budget runs
     again with half the states per lane, down to 512 (`Error::BackendBudget`). Fewer lanes would
     not help: the summed arenas grow.
   - **Launch bound.** It is 200, up from 99 (user decision, 2026-11-03), because the wave doubled
     to 158 clauses.
   - **No CPU fallback** (user decision).

## Consequences

- S1 lowers 158 of 286 clauses, up from 77, and all 158 are exact on the 60 fixtures.
- What is still refused:

  | Reason | Clauses |
  |---|---|
  | shared binding | 32 |
  | arguments binds | 22 |
  | parse | 17 |
  | resolves.binding | 14 |
  | constraints | 13 |
  | unknown fact under not | 12 |
  | compare | 6 |
  | formatArgs | 6 |
  | utility call | 3 |
  | binds | 2 |
  | binds occurrence or redefines | 1 |

- The wave has 158 clauses and 1,162 labels.
- Two clauses (`ruff-b006`, `ruff-f633`) need the smaller states: at NARROW, pending, captures and
  cursors alone are about 284 MB, over the 256 MB budget, before any input (O35).
- The fact rows are rebuilt per run, not cached, as decision 0026 does for the families.

## Measured

At load 47; CPU is user + sys.
- **S1** (`lowered_python_clauses_match_where_ws_matcher_does`): 158 of 286 lowered, 158 exact on
  60 fixtures. The whole test took 3.90 s of CPU.
  - Inside it, W's matcher took 2.25 s and the lowered clauses 1.39 s (wall in the test).
  - Making the fact rows took 0.14 s. Before the per-file scope table it took 13.1 s.
- **S2** (`lowered_clauses_find_the_same_roots_on_metal`): 158 of 158 equal; 1,577 launches, at
  most 51 for one clause. The whole test took 9.11 s of CPU (3.57 user, 5.54 sys).
- **Targeted tests:** `waves::` 96 of 96, `matcher::` 389, `compile::` 317, and
  `compile_matcher_tests` 6. `the_backends_find_the_same_keys_on_real_files` is ignored (O35).

