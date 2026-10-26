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

### Compact tape projection and indexed native baseline

The second reference removes the per-node AST maps and unused structural relations.
It reads tape columns once and creates nodes only for selected call neighborhoods and context boundaries.
Binding targets use direct tape access and a span search instead of a copied global range index.
Eleven plan tests and five semantic projection tests pass.
The indexed native oracle also agrees with the stateless native oracle on the positive fixtures.

The 500-file median reference CPU fell from 779 ms to 44 ms.
Structural collection fell from 737 ms to 11 ms.
The recorded process peak footprint fell from about 702 MB to 136 MB.

The full frozen corpus contains 5,725 Python files and 95,697,853 source bytes.
All three reference runs agree with native, with complete coverage and no findings.
This remains a negative workload; positive-result correctness comes from the fixtures.
Median reference execution CPU is 843 ms versus 1,394 ms for indexed native matching.
The approximately 1.65x improvement does not meet the proposed 2x execution gate.
Native first-use candidate selection costs another 43.879 seconds; later runs reuse that frontier.
Those costs are reported separately and must not be mixed into a matching speedup claim.
Common parse/tape construction costs 9.787 seconds; mandatory index construction costs 58.235 seconds.
These costs dominate end-to-end time and are not improved by this reference.

The full reference reads 19,007,630 tape rows and retains 65,399 async calls.
It makes 65,302 semantic requests and builds 1,063 scope tables.
Scope construction costs a median 462 ms, the largest remaining reference stage.
Two SQL executions use 2,030,306 VM instructions and about 55 ms of CPU.
Only ten candidate branches and thirteen value origins survive.
This is evidence of excessive semantic demand and relation input, not expensive final composition.
The next change applies a conservative import-syntax screen before qualification-only requests.
Binding and value requests retain their exact semantic behavior.

Artifacts: `reference-v2-500.json`, `reference-v2-full.json`, and `reference-v2-manifest.json` in `benchmark/shared-structural-execution`.
The full benchmark process peak footprint is 1,462,961,664 bytes, including both engines and common preparation.
These results qualify an independent reference only. Generic compilation and production cutover remain pending.
