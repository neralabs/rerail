# Shared structural execution journal

## 2026-10-26 — qualification implementation starts

The accepted plan is [sql-structural-execution-plan.md](sql-structural-execution-plan.md).
The source plan came from `jeremy/sql-capture-demand-spikes` at `b3e97cd4`.
Implementation uses `jeremy/shared-structural-execution`, based on integration commit `85098ee3`.
The sibling integration worktree has uncommitted changes; this branch does not include them.

The integration base contains the anchor changes but not the packed compiler or shared fact executor.
The first implementation therefore supplies an independent reference for the complete four-rule async-call family.
It keeps the existing module boundaries: `query/plan` for composition and `matcher/python_facts` for semantic projection.

The reference must include ASYNC210, ASYNC221, ASYNC222, and ASYNC230.
Spawn modes, binding values, destructuring, and `Path` receivers are part of this gate.
The existing matcher serves only as a comparison oracle.
The reference cannot call it to evaluate a condition.

The reference remains in the qualification build until the compiler and catalog gates pass.
This is not a production engine selection switch.
The final dispatcher changes once, as specified in the plan.

Implementation work:

- Add atomic publication of findings per rule and document.
- Keep logical uncertainty separate from resource failure.
- Expose completed integer relations to SQLite without row-by-row INSERT statements.
- Collect shared Python call and execution-context facts in one document traversal.
- Project import and binding facts from an explicit scope table, without rule callbacks.

Validation results and measurements will follow here after execution.

### First executable reference and measured failure

Fourteen focused tests passed: five semantic projection tests and nine contract, structure, and complete-family tests.
The tests use the production tape representation for the complete family.
Sixteen copies of the labels leave structural visits, semantic requests, scope builds, and origin rows unchanged.
No accepted semantic differences were needed for these fixtures.

The checks caught and fixed these errors before measurement:

- Raw method spelling differs from normalized semantic identifier spelling.
- A parenthesized receiver does not satisfy a direct identifier condition.
- Destructuring indices require eligible list containers.
- Leading whitespace is outside some AST root spans; locations must use document coordinates.
- Explicit line continuations are not callee expressions.
- A comment can break an explicitly required adjacent-sibling relation.

The first 500-file run used 6,785,917 source bytes.
It found no violations; it measures a negative workload, not positive-result throughput.
The fixture gate above covers positive findings separately.

Median reference execution CPU was 0.779 seconds, including 0.737 seconds for structural fact collection.
SQL used two executions and 79,553 VM instructions.
The producer inspected 1,405,851 syntax nodes, found 57,113 calls, and retained 3,584 async calls.
It submitted 3,575 distinct semantic requests and built 75 scope tables.
The process peak footprint was about 702 MB, including both benchmark implementations and corpus preparation.

This exposes the next structural error: broad AST object materialization dominates this reference.
The correction is a compact tape pass with narrow call-fact output and on-demand node access.
It must replace the reference's broad collection path before full-corpus qualification.

The stateless native comparison took a median 8.650 seconds of CPU.
That comparison rebuilds scope tables per clause and includes index construction.
It is not the indexed native execution gate and does not establish the target speedup.
A second comparison now uses the production anchor index and shares native scope tables across all four clauses.

Artifacts: `benchmark/shared-structural-execution/reference-500.json` and its manifest.
The manifest records source hashes, binary hash, corpus commit, and compiler settings.
This qualification build uses optimization level 3, 16 local codegen units, and no LTO.
Apple's linker rejected the initial cached LLVM bitcode; disabling LTO resolved the local build issue.
