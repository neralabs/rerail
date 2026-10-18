# Packed executor port plan

> **Status:** Superseded as a complete rule-engine plan by
> [SQL rerailer revival design](sql-rerailer-revival-design.md).
> The packed executor now produces terminal and fact relations for SQL composition.
> See [NER-1562 packed executor landing pad](ner-1562-landing-pad.md) for the active port sequence.

The detailed architecture and lowering design is in
[`packed-executor-architecture.md`](packed-executor-architecture.md).
It defines both execution pipelines, their compatibility boundary, and direct lowering for Rerail rules.

## Decision

Do not merge the NER-1562 Landing branch into Rerail as one change.

Port the executor as a new backend behind Rerail's current matcher.
Keep the current matcher as the semantic oracle during the port.
Use compile-time routing for rules that the new backend does not support.
Never use runtime fallback after a backend starts a rule evaluation.

Use the packed Rust interpreter as the semantic oracle.
Port the Metal interpreter and Metal JIT as the first production backend.
Port the CPU JIT later for Linux, Windows, and unsupported macOS systems.

Do not port the Landing result schema or content store without new measurements.
Rerail and Landing now have different result workloads and storage designs.

## Source examined

This plan compares these revisions:

- Rerail: `d81f2749`
- Landing NER-1562: `87ae98370`
- Landing branch base: `origin/jbarnes/ner-1562-scale-test-the-re-railer`

The Landing branch has 170 commits after its remote base.
The compared Landing execution and storage files have approximately 41,000 lines.
The equivalent Rerail files have approximately 10,000 lines.

The Landing head is not a safe copy source yet.
`tempo/docs/shared-arena-direct-publication-design.md` says that production ABI v4 integration is incomplete.
The last commits remove ABI v3 and change Rust and Metal arena references.
The branch does not record completed ABI v4 acceptance gates.

The latest 640-file production profile took 52.48 seconds and reached 3.19 GB RSS.
A summary-only restore reduced reopen time to 3.17 seconds.
The shared-arena ABI targets the remaining memory and publication costs.

Use Landing commit `4dff797b4` as the initial pre-v4 reference.
This revision still has the measured ABI v3 memory limits.
Reassess the Landing head after ABI v4 passes its documented selection gates.

## What Landing adds

The Landing work adds these main capabilities:

1. A complete packed execution batch for files, programs, roots, and immutable tables.
2. A callback-free Cranelift CPU JIT.
3. A one-dispatch Metal interpreter and Metal JIT.
4. Native structural, regex, Lua, capture, reduction, and output programs.
5. Bounded compilation caches and reusable execution workspaces.
6. Per-evaluation outcomes with explicit work and capacity failures.
7. Operation-specific arena plans instead of one capacity estimate.
8. Normalized result rows, summary-only reopen reads, and keyset paging.
9. Query-independent FTS indexes for candidate selection.
10. Detailed phase, memory, capacity, and parity measurements.

The architecture is valuable.
The Landing compiler and storage schema are not direct replacements for Rerail components.

## Important differences

### Rule language

Landing optimizes `Program` values lowered mainly from Tree-sitter SCM patterns.
Rerail runs `rerail.policy/v1` rules through `GrammarMatcher`.
SCM must remain a separate frontend, not Rerail's intermediate language.
Rerail patterns must lower directly from Tree-sitter-parsed ast-grep `PatternNode` values.

Rerail's matcher supports utilities, relations, bindings, counts, scopes, values, calls, and transforms.
The Landing `Program` type does not represent all these operations.

The repository contains 541 rule documents with 1,151 AST clauses.
These clauses use nested operations such as `matches`, `has`, `stopBy`, `inside`, `count`, and `nthChild`.
The shipped complexity pack uses utility references, bounded descent, regex tests, and count reduction.

A direct replacement of `GrammarMatcher` with Landing's SCM compiler will change rule behavior.

### Tape format

Both repositories identify their persisted tape as version 4 with the `TMPTAPE4` magic value.
The formats are not interchangeable.
Rerail's final header table stores Blade syntax-error ranges.
Landing's final header table stores Tree-sitter supertype entries.
Use Rerail's restored `TapeDoc` during the first executor port.
Then replace both formats with generic tape version 5.
Version 5 stores one tree over one source, generic node classes, and parser recovery as nodes.
Blade template syntax and generated PHP use separate tapes with a workspace source map.

### Production execution

Rerail's production path is `native/src/code_rule_dsl/workspace/evaluate.rs`.
It builds one shared file anchor index and runs `matcher::run_clause` for each candidate clause.

Rerail's current `transducer::Program` is a benchmark and test path.
The production workspace does not execute it.

Landing changed its production workspace to batch every `Program` for a file.
It then selects the packed backend once for that batch.
This production connection is the main integration seam for Rerail.

### Storage

Rerail has a modular content-addressed store in `native/src/content_store/`.
It stores bounded local results in compressed frames.
It also stores candidate facts and query-independent kind FTS data.

Landing has one large `content_store.rs` and normalized local result relations.
The normalized schema was necessary for millions of SCM highlight and tag rows.
Rerail normally stores bounded policy violations instead.

Do not replace Rerail's store with the Landing file.
Port isolated storage changes only after a Rerail profile identifies the same cost.

### Platforms

Rerail ships native engines for macOS, Linux, and Windows on arm64 and x64.
Metal can improve only supported macOS systems.

This platform limit does not require CPU-first development.
Platforms without Metal use the packed CPU JIT backend. They do not use the semantic matcher.

## Port boundary

Create a new module below `native/src/code_rule_dsl/packed/`.
Keep the module independent from N-API, SQLite, and workspace state.

The module owns these items:

- A platform-neutral packed ABI.
- Immutable file, tape, program, and task tables.
- Checked mutable arenas.
- A Rust interpreter for parity.
- Metal interpreter and Metal JIT backends behind a macOS feature.
- A later callback-free CPU JIT.
- Explicit outcomes and capacity errors.
- Packed result views.

The existing compiler owns rule meaning.
The SQL compiler lowers complete `GrammarMatcher` clauses into relational plans.
During development, the terminal compiler lowers completed operations into test-only packed programs.
The matcher is a test-only semantic oracle. Production is SQL-first and fails unsupported packed lowering without fallback.

Partial coverage is a migration state, not a product capability.
Every valid `rerail.policy/v1` AST clause must lower without a host callback.
A new public operation must include its SQL lowering and each required terminal implementation.

The final routing decision selects a terminal backend for one relational plan.
It does not select a semantic subset.
The selected backend becomes part of the compiled execution identity.
It must not change during one evaluation.

## Semantic contract

The current matcher is the oracle until the port is complete.
For the same file, clause, limits, and selected roots, both paths must produce equal data:

- Completion or failure status.
- Violation order and multiplicity.
- Source spans and anchors.
- Capture labels, values, and order.
- Messages and fixes.
- Suppression behavior.
- Work, witness, and source limits.
- Behavior under syntax errors.

Capacity exhaustion is an evaluation failure.
It must not produce partial violations.
It must not start the current matcher as a fallback.

The packed path can use a test-only adapter to construct current `Witness` values.
Remove this adapter only after direct result publication passes differential tests.

## Port phases

### Phase 0: Reproduce the Rerail baseline

Add a benchmark that uses production Rerail rules and the production workspace.
Use a pinned corpus near 2 million lines of code.

Record these values in machine-readable output:

- Corpus revisions, files, bytes, lines, and languages.
- Rule and clause counts.
- Full-scan wall time and CPU time.
- Incremental p50, p95, and p99.
- Peak and steady RSS.
- Content-store bytes.
- Parse, tape, selection, match, render, read, and write times.
- Candidate roots, matcher calls, violations, and failures.

Keep a small CI guard.
Keep the full corpus benchmark as an explicit developer command.

**Gate:** The benchmark runs again from pinned inputs and reports all required fields.

### Phase 1: Import the backend-neutral ABI

Port the stable Landing packed records and range checks.
Rename Tempo terms to Rerail terms during this phase.
Do not copy the Landing workspace or content store.

Use Rerail's `TapeDoc`, grammar identifiers, and source ownership.
Add size, alignment, offset, overflow, and corrupt-range tests.

**Gate:** The packed module builds on all six release targets.

### Phase 2: Port the packed Rust interpreter

Port the packed scheduler and operation-specific arenas.
Keep all task memory bounded before execution starts.
Do not allocate `Vec`, `HashMap`, or JSON values inside task execution.

First lower a small structural subset:

- Kind and field tests.
- Exact and regex text tests.
- Child, descendant, sibling, and parent navigation.
- Ordered alternatives.
- Captures and one violation output.

Run the packed interpreter and `matcher::run_clause` on identical selected roots.
Compare their complete outputs and failures.

**Gate:** Every supported rule passes differential tests and corpus parity.

### Phase 3: Cover one shipped rule class

Add the operations required by the complexity pack:

- Utility references.
- `any` alternatives.
- `has` with `stopBy`.
- Count reduction with an offset.
- Message interpolation from the count binding.

Keep the reduction in packed typed records.
Do not convert every terminal result to `NodeMatch` before the reduction.

**Gate:** All three shipped complexity rules use the packed interpreter with exact parity.

### Phase 4: Port Metal

Port the Landing Metal interpreter, Metal JIT, and reusable workspace pool.
Use the same ABI and outcomes as the packed Rust interpreter.

Require one invocation and zero host events for a non-empty batch.
Keep workspace storage reusable and demand-zero.
Do not use page eviction or coarse program partitioning.
Landing measurements show that both approaches increase latency.

Cache pipelines by the backend format, device, options, and ordered program identities.
Bound the cache by entries and bytes.

**Gate:** Both Metal backends match the packed Rust interpreter and current matcher.

### Phase 5: Connect shadow batching

Group completed clauses by file after candidate selection.
Build one packed batch for the shadow comparison.
Preserve stable file, clause, root, and output order.

Keep the current matcher as the only publishing path.
Do not parse or restore a tape twice for the shadow batch.
Do not rebuild immutable program tables for each file.

Run Metal JIT in shadow mode on supported macOS systems.
Keep the current matcher on other systems during this phase.

**Gate:** Supported macOS systems can compare one Metal call per shadow batch without changing published results.

### Phase 6: Complete rule coverage

Add one semantic family at a time:

1. Positional relations and `nthChild`.
2. Metavariable equality and transforms.
3. `each`, nested selections, and reductions.
4. Scope and value operations.
5. Calls and cross-file relations.
6. Fix rendering and suppression directives.

Each family needs exhaustive lowering over its public operation variants and differential fixtures.
Do not remove the old path one family at a time.

**Gate:** Every valid public AST operation has a packed lowering, and every repository rule lowers successfully.
The inventory reports zero unsupported operations.

### Phase 7: Port the callback-free CPU JIT

Port `native_ir.rs` and the Cranelift lowering.
The generated function receives packed arrays and checked arena ranges.
It must not call Rust matcher helpers.

Cache modules by backend format, target, options, and ordered program identities.
Bound the cache by entries and bytes.

Select the CPU JIT where Metal is unavailable.
Do not change the Metal selection on supported macOS systems.

**Gate:** The CPU JIT matches Metal, the packed Rust interpreter, and the current matcher.

### Phase 8: Reassess ABI v4 and direct publication

Reassess Landing ABI v4 after its acceptance report is complete.
Port shared evaluation arenas if Rerail profiles show retained workspace growth.

Keep Rerail's result frames unless storage dominates the measured workload.
If storage dominates, first add summary-only restore and prepared identifier caches.
Consider normalized rows only when result paging or result volume requires them.

**Gate:** Each storage change improves a Rerail benchmark and preserves reopen parity.

## Changes to avoid

Do not cherry-pick all 170 Landing commits.
The repositories renamed and changed the same compiler, workspace, and storage code.
A mechanical merge will hide semantic changes.

Do not make SCM the internal Rerail rule language.
It does not cover the current `rerail.policy/v1` contract.

Do not leave Metal as the only fast path permanently.
Most release targets cannot use it, but this constraint does not block a Metal-first port.

Do not normalize all result storage before measurements.
Landing's result volume is not representative of bounded Rerail violations.

Do not remove the current matcher early.
It is the only complete oracle for the Rerail rule language.

Do not retry a capacity failure with another backend.
This changes observable failure behavior and hides planner errors.

## Acceptance gates

The final executor must meet these gates:

1. Current and packed paths have exact semantic parity.
2. The packed Rust interpreter and Metal have exact packed-record parity.
3. The later CPU JIT has parity on every release target.
4. Every valid `rerail.policy/v1` AST clause lowers to a verified packed program.
5. Backend failures never cause runtime fallback.
6. Initial worker peak RSS is less than 500 MiB.
7. Steady-state RSS is less than 1 GiB.
8. Incremental p50 is less than 100 ms.
9. Incremental p95 is less than 500 ms.
10. The projected store for 100 similar workspaces is less than 10 GB.
11. Linux, Windows, and macOS release jobs pass on arm64 and x64.
12. Reopen and incremental edits preserve result hashes and failure records.

## Recommended first pull request

The first pull request must contain only foundation work:

1. Add the reproducible production benchmark.
2. Add a rule-operation inventory report.
3. Add the empty `packed` module and ABI records.
4. Add target layout and corrupt-range tests.
5. Record the Landing source commit in the module documentation.

This change gives the port a measured baseline and a narrow review boundary.
It does not change production rule execution.
