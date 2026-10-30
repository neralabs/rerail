# 0011: Composite matchers lower to node-label passes plus positive walking programs

Status: accepted
Date: 2026-10-29

## Question
06-first-slice S1: how does W's composite matcher lower to packed transducer programs? The matcher
operators are `all`, `any`, `not`, `has`, `inside`, `precedes`, `follows`, `field`, `stopBy`,
utilities (recursive ones included) and `constraints`. No stream has this lowering. G's
`AstMatcher::Program` wraps one compiled pattern, and N lowered composites to relational SQL (tree
axes in SQL, not copied). The packed IR is a path machine: forking paths, positive tests, and
loops the verifier admits only when they move forward in preorder (`cycle_progress`). It has no
negation, no backward sibling step, and no backward loop.

## Decision
A clause becomes a sequence of **node-label passes**, then one root program
(`transducer/packed/compile/matcher.rs`).
- **Passes.** Each pass computes a set of nodes over the tape: a full-width phase.
- **Tests.** A later program tests a label with `SUPERTYPE`, as a derived grammar class
  (`TapeDoc::with_labels`, one bitset per class from `DERIVED_CLASS_BASE`). No new operation, so
  rule F1 holds for the ops.
- **Labels come from:**
  - an ast-grep pattern (`PackedPattern`), optionally with constraints checked on its single-node
    captures;
  - a program's roots;
  - the nodes a program's walk reaches;
  - a complement;
  - the empty set.
- **Negation is a complement label.** `not` and a stop rule's bound are evaluated between passes,
  never inside a program.
- **Programs are positive.** They test kinds and regexes inline and walk one relation deep:
  - `has`: `CHILD` or `DESCEND`, or a `CHILD` loop bounded by the stop label.
  - `precedes`: `NEXT` or `FOLLOWING`, or a `NEXT` loop.
  - `inside` neighbour: `PARENT`.
  - `field`: `FIELD`.
  - A relation's rule that itself walks becomes a label.
- **Backward walks become forward passes.** `inside` (`end` or a stop rule) and `follows` are the
  nodes a forward walk reaches from each match, so every loop verifies.
- **Recursive utilities are groups** of passes iterated until none changes. Positive recursion
  takes the least fixpoint. Recursion through `not` is admitted only when every walk on the
  recursive path moves forward in preorder (`has`, `precedes`). Each node's verdict then reads only
  later nodes' verdicts, so the fixpoint is unique and the rounds reach it. Only labels that read
  the recursion are iterated; the rest are passes of their own.
- **Each pass starts from its domain:** the kinds its rule needs (`MatchTree::kinds`), or the label
  its walk starts from.
- **Refused by name, counted by the S1 gate:**
  - bindings shared between leaves;
  - semantic predicates;
  - `count`, `nthChild`, parameterized utilities, comparisons;
  - constraints on variables bound below the top level;
  - non-`smart` patterns.

## Prior work consulted
- **Spike history 12(d) and design-index top-15 #14:** "Leave negation, recursion and aggregation
  to SQL composed around the structural fragment"; "negation, counting and closure belong in caller
  SQL". *Agrees:* the transducer keeps the positive structural fragment, and negation and recursion
  are compositions between passes. They are bitset operations on the CPU here; as relations in the
  slice's wave they are SQL (`EXCEPT`, waves).
- **Design-index 7:** an arbitrary-depth descendant is an interval test on preorder plus size, so
  no recursion is needed; recursion belongs to waves. *Agrees:* `inside`/`has` to the end are
  single `DESCEND` passes. The `inside` targets are preorder intervals, which a later step can use
  in place of the walk.
- **Spike 2.3 (TAG L16-41):** ast-grep over the tape is exact (4,600/4,600 pairs). *Agrees:*
  pattern leaves use the packed pattern over the same tape, and W's matcher on that tape is the
  oracle.
- **Spike 2.17:** position parity on node sets, mutation-checked; never count an unanswered query
  as clean. *Agrees:* the gate compares root sets per clause and fixture. A refused clause is
  counted, not skipped silently. `the_gate_catches_a_widened_walk` shows that a wrong lowering
  fails.
- **Spike 2.15 / design-index 13:** sharing was revised three times ("Everything a batch could
  share, it should not", S149). *Partial:* labels are not hash-consed across clauses yet (Phase
  4c). When they are, the sharing must be measured, not assumed.
- **Spike 2.23:** generic interpretation is about 12× slower than specialized code. *Agrees:* the
  gate's lowered passes are about 15× W's matcher on the CPU interpreter in a debug build. That is
  the interpreter's cost, which the JIT binding (Phase 4d) and the prefilter's anchors (S4) address.
- **Bend investigation (sibling session, BendRT/BendTT papers, not measured on Rerail):**
  - *Adopted:*
    - "sequence full-width phases, do not fork them against each other": barriers are passes;
    - structural descent as the termination proof: forward-only recursion through `not`, and
      forward-only loops;
    - one sequential program per root, with FORK used only as `any`;
    - every binding must give the same keys: the CPU-against-Metal check is S2's.
  - *Not adopted:* Bend's join with slots and a countdown, which would put divergent nested
    searches in each lane.

## Consequences
- The S1 gate result (`lowered_python_clauses_match_where_ws_matcher_does`): 77 of 286 Python
  `ast` clauses lower, and all 77 match W's matcher exactly on all 475 fixtures. The remaining 209
  are refused by operator (`06-first-slice.md` S1).
- **Metal needs the labels.** Labels are tape data the CPU interpreter reads. For Metal, the
  `with_labels` bitsets must be uploaded with the tape's class table, and the passes run as kernels
  in one command buffer (no host round trip per pass). Recorded as open item O23.
- **Semantic predicates** (`resolves` 94, `parse` 13, `arguments` 13, `resolves.binding` 11 and
  others) become labels from fact producers (`transducer::semantic_facts`). That is outside the
  operator lowering, recorded as O24.
- **`count`** (25) needs a counting pass (a per-node count label compared with a bound): S2.
