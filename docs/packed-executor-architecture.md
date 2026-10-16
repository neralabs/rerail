# Packed executor architecture and Rerail lowering design

> **Status:** This document remains the semantic inventory for packed terminal execution.
> It no longer defines the higher-level rule engine.
> [SQL rerailer revival design](sql-rerailer-revival-design.md) defines SQL composition.
> [NER-1562 packed executor landing pad](ner-1562-landing-pad.md) defines the active port boundary.

The exact pattern lowering design is in
[`rerail-pattern-lowering-design.md`](rerail-pattern-lowering-design.md).
The parser-independent tape design is in
[`generic-tape-v5-design.md`](generic-tape-v5-design.md).

## Decision

Rerail must keep `rerail.policy/v1` as its only rule frontend.
Do not port Landing's SCM parser, compiler, catalog, or production wrappers.
Landing added SCM only to drive the executor with a large external query corpus.

The port must use Landing's verified `Program` boundary.
The port must extend that boundary for Rerail semantics.
Production executors must receive only verified programs and packed input data.
They must not parse policy JSON, SCM, regex source, or ast-grep patterns.

Tree-sitter has three roles in the new path:

1. It parses each source file before tape construction.
2. It parses language-specific ast-grep patterns during policy compilation.
3. Its grammar resolves kind and field names to numeric identifiers.

The Rerail compiler must lower the parsed rule directly to `Program`.
It must not print an SCM query and parse that query again.
SCM remains historical test infrastructure on the Landing branch.
It is not part of the Rerail port.

## Architectural boundary

The target boundary is:

```text
rerail.policy/v1
  -> validation and policy compilation
  -> ordered Rerail rule graph
  -> dialect-specific Program lowering
  -> Program verification
  -> compiled route: Current or Packed(Program)

source text
  -> Tree-sitter or Forte parser
  -> Rerail TapeImage
  -> Rerail TapeDoc
  -> packed file tables

Program + packed file tables + candidate roots + limits
  -> Rust interpreter, Metal interpreter, Metal JIT, or CPU JIT
  -> packed clause outcome
  -> Rerail violation publication
  -> existing Rerail content store
```

`Program` is the only semantic input to an executor.
Packed file tables are workload data, not semantic instructions.
The program declares each required side table.
The verifier rejects a program when its selected backend lacks a required operation.

## Current Rerail architecture

### Compilation path

The public entry point is `CodeRuleDslRuntime::compile` in `native/src/code_rule_dsl/mod.rs`.
The path has these stages:

1. `parse_document` reads JSON or YAML and rejects duplicate object keys.
2. `config::resolve` expands policy configuration and definitions.
3. `compile_document` validates the policy, scope, mode, clauses, and capabilities.
4. Each AST query compiles once for each supported grammar dialect.
5. `compile_ast_rule` builds an ordered `AstMatcher` graph.
6. Static analysis builds anchor, required-text, capture, utility, and soundness plans.
7. Violation compilation builds anchors, templates, fixes, suppression rules, and projections.
8. The result becomes an immutable `CompiledCodeRule`.

The compiled policy contains more than an AST matcher.
It also contains scope, limits, cross-file queries, placement, and violation behavior.
These parts affect observable results.

### Rerail matcher model

`GrammarMatcher` is the per-dialect execution unit.
It contains these items:

- An ordered `AstMatcher` graph.
- An `AnchorPlan` for candidate selection.
- A required-text filter.
- A syntax-error soundness result.
- A utility table.
- An optional file condition.
- A three-valued execution flag.
- An optional post-match transform.

`AstMatcher` supports these structural operations:

- `Pattern`, `Kind`, and node-text `Regex`.
- `All`, `Any`, and `Not`.
- `Inside`, `Has`, `Precedes`, and `Follows`.
- Relation fields and `stopBy` modes.
- `NthChild` and `ofRule`.
- Utility references and parameterized utility calls.
- Constraints on captured nodes.
- Capture comparison and count reduction.

It also supports Rerail semantic operations:

- Resolution, values, arguments, and bindings.
- Parsing of literals and host tests.
- Capture, formatting, and layout tests.
- Call-graph tests and file scopes.
- Transforms and exported values.

The term order is semantic.
Bindings flow through terms in compiler order.
`All` uses a transactional environment.
`Any` commits the first matching branch.
Relations commit the first related match in walk order.
`Not` reads existing bindings but commits no new bindings.

Rerail also supports unknown values.
An unknown result is different from false.
The three-valued rules affect `All`, `Any`, `Not`, relations, stop rules, and counts.

### Rerail file execution path

The production path is `native/src/code_rule_dsl/workspace/evaluate.rs`.
It performs these operations:

```text
workspace request
  -> candidate dispatch by path, language, and stored facts
  -> stored result, source, and tape restoration
  -> line table construction
  -> regex-clause execution
  -> source parse when no current tape exists
  -> TapeImage encoding and TapeDoc restoration
  -> syntax-error classification
  -> required-text filtering
  -> one shared FileAnchorIndex
  -> roots selected for each GrammarMatcher
  -> matcher::run_clause for each candidate AST clause
  -> capture transforms and violation rendering
  -> suppression filtering
  -> witness and failure publication
  -> local result persistence
  -> cross-file producer and consumer phases
```

`run_clause` visits candidate roots in preorder.
It applies root-kind filters before matcher execution.
It preserves one utility-work counter for the clause.
It stops on a utility limit, deadline, or visitor request.

`violation.each` has special behavior.
Rerail matches the same root again after each accepted binding.
Each new run excludes the binding ranges already reported.
This process preserves distinct bindings from one root.

Violation construction is also semantic.
It selects an anchor, applies transforms, renders templates, and builds a fix.
It then checks syntax-error overlap and suppression directives.
Suppressed findings do not consume the witness allowance.

### Rerail workspace and persistence

The workspace keeps files, candidate sets, tapes, witnesses, failures, and cross-file state.
The content store uses content-addressed sources and tapes.
It stores bounded local results in compressed frames.
It also stores candidate facts and query-independent kind indexes.

Cross-file work has producer and consumer phases.
AST relations publish projected rows.
Require-each and budget clauses consume those rows.
Call analysis also builds per-file summaries before dependent matches run.

This architecture means that a clause is not always a file-local tree query.
Some clauses need prepared workspace data or a later consumer phase.

## Landing and Tempo architecture

### Frontend path

Landing accepts SCM as one frontend.
Its SCM compiler performs these operations:

1. Parse SCM into a typed query syntax tree.
2. Resolve node kinds, fields, captures, predicates, and directives against a grammar.
3. Preserve Tree-sitter pattern order, repetition, and capture semantics.
4. Lower the typed query into a fixed-width `Program`.
5. Verify all program tables and control-flow targets.

The executor never reads SCM.
SCM source does not cross the `Program` boundary.

Landing also has a very small ast-grep frontend.
That frontend accepts only a root terminal with Smart strictness.
It rejects structural ast-grep patterns and metavariables.
Thus, it is not a usable Rerail frontend.

### Landing `Program`

The current `Program` contains these immutable tables:

- A fixed-width structural instruction stream.
- Interned byte strings and capture labels.
- Long operands.
- Node-text `ValueProgram` values.
- Post-capture `CaptureProgram` values.
- Output directives in an `OutputProgram`.

The structural stream supports these instruction groups:

- Tests: kind, field, flags, text, size, assertion, first, last, and absent field.
- Navigation: child, next sibling, parent, descendant, and following sibling.
- State: save cursor, restore cursor, capture, and repeat state.
- Control: fork, jump, stop, and emit.
- Tree-sitter-specific supertype tests.

`ValueProgram` supports exact text, prefix text, a byte NFA, and lowered Lua patterns.
Executors do not call the Rust regex engine or a Lua runtime.

`CaptureProgram` supports these post-capture tests:

- Capture equality.
- Text, set-membership, kind, and substring tests.
- Ancestor or descendant kind relations.
- Same-line tests.

`OutputProgram` supports static and dynamic properties.
It also supports offsets, stripping, adjacent selection, downcasing, and MIME language mapping.

### Landing execution model

A task is one program, one file, and one candidate root.
The program explores ordered NFA paths from that root.
Each path contains these values:

- A program counter and current node.
- The starting root.
- Persistent capture, cursor, and repeat states.
- A work count.

The scheduler removes duplicate query states.
It uses capture antichains to retain semantically distinct paths.
It also implements greedy repetition with replaceable scheduled paths.

Execution produces ordered results.
Each result records its task ordinal and local ordinal.
It also records captures, properties, transformed bytes, and task status.

### Packed batch and backends

`PackedBatch` concatenates immutable data for many files and programs.
It contains these main table groups:

- Nodes, child ordinals, source bytes, line starts, and supertypes.
- Program words, strings, labels, values, assertions, and output instructions.
- Files, programs, tasks, and evaluations.
- Mutable arenas for paths, links, scheduler state, results, and output bytes.

The planner assigns bounded capacity before execution.
Each evaluation also receives a work limit and arena quotas.
An executor reports work exhaustion, capacity exhaustion, or backend failure explicitly.

All backends consume the same packed records:

- The Rust interpreter is the semantic reference for packed execution.
- The CPU JIT lowers verified operations to callback-free native code.
- The Metal interpreter reads packed instructions on the GPU.
- The Metal JIT specializes execution for an ordered program set.

Landing's production workspace groups all `Program` matchers for one file.
It sends one packed batch to the selected backend.
The workspace then converts outputs to matches and normalized result rows.
Non-program matchers remain in the older root loop.

## Why the two implementations diverged

### The work split across two branches

The advanced executor did not evolve inside the current Rerail repository.
Its first fixed-word tape executor landed on the Landing branch on September 27, 2026.
The SCM and packed-backend work then continued on that branch.

The standalone Rerail repository imported its core from Landing main on October 8, 2026.
That import included the small benchmark transducer, but not the advanced feature-branch history.
Rerail later imported selected policy features instead of the executor branch.

The result is parallel development from a related base.
It is not one clean branch with an old and a new executor.
Both sides changed shared concepts for different workloads.

### Rerail optimized policy meaning

Rerail development concentrated on the `rerail.policy/v1` contract.
The main changes added or preserved these features:

- Complete ast-grep rule ordering and capture behavior.
- Three-valued semantic operations.
- Counts, utility calls, comparisons, and transforms.
- Scope, binding, value, resolution, and call analysis.
- Cross-file producer and consumer phases.
- Policy findings, fixes, suppression, placement, and modes.
- Language-family adapters and explicit language refusals.
- Forte-based Blade parsing and Blade error ranges.

These features made `GrammarMatcher` more expressive.
They did not make the benchmark transducer a production path.

Rerail's normal result is a bounded policy violation.
This workload favored compact result frames and existing workspace semantics.
It did not require millions of generic capture rows.

### Landing optimized a large structural-query workload

NER-1562 needed a repeatable scale workload before a transducer supported all Rerail rules.
The branch used large, existing Tree-sitter query catalogs stored as `.scm` files.
Those catalogs supplied thousands of grammar-specific structural patterns and many matches.

This workload drove a different sequence:

1. Build a fixed-word structural transducer over persisted tapes.
2. Connect SQL and anchor indexes to sparse candidate roots.
3. Convert complete SCM patterns into verified programs.
4. Match native Tree-sitter ordering, quantifiers, captures, predicates, and directives.
5. Move text and capture predicates into callback-free value programs.
6. Pack files, programs, roots, and arenas into one backend ABI.
7. Add Metal and CPU JIT backends.
8. Normalize high-volume results and add keyset result pages.

An early SCM conversion proof was removed on September 28.
It proved that patterns parsed and executed, but it did not prove full semantic parity.
The branch then added a typed SCM parser and exact lowering rules.

SCM requirements explain several Landing changes:

- Query supertypes required persisted supertype paths.
- Native quantifiers required repeat state and greedy scheduling.
- Query captures required ordered capture occurrences and containment reduction.
- Neovim predicates required Rust regex and Lua-pattern value programs.
- Query directives required output programs and dynamic properties.
- High result counts required normalized rows and result paging.
- GPU execution required bounded arenas and explicit task outcomes.

These are good executor components.
They do not represent the full Rerail policy model.

### The same names now hide different contracts

Both sides still use names such as `TapeDoc`, `GrammarMatcher`, `Program`, and `workspace`.
The shared names can make the branches appear closer than they are.

On Rerail, `GrammarMatcher` is the semantic execution unit.
Its environment, utility tables, unknown values, and violation behavior are central.

On Landing, `Program` became the accelerated execution unit.
Its ordered NFA paths, packed tables, and generic outputs are central.

On Rerail, the workspace coordinates semantic and cross-file policy phases.
On Landing, the changed workspace also batches programs and publishes high-volume normalized matches.

The tape divergence has the same cause.
Rerail used its final version-4 table for Blade syntax errors.
Landing used that position for SCM supertype paths.
Neither branch incremented the common tape identity after its local change.

## SCM as a language and frontend

### What SCM means here

SCM is the conventional file format for Tree-sitter queries.
It uses an S-expression syntax that describes tree shapes.
The `.scm` suffix refers to Scheme-like syntax, not a Scheme evaluator.

A typical query has this form:

```scheme
(function_declaration
  name: (identifier) @name
  parameters: (formal_parameters) @parameters) @function

(#match? @name "^[A-Z]")
```

The first form describes a tree shape.
Captures attach names to matched nodes.
The predicate reads one or more captures after structural matching.

Tree-sitter query syntax and a host's predicate library are separate layers.
Tree-sitter defines the structural query model and exposes predicate records.
Applications define many predicate and directive meanings.
Neovim, Helix, and other providers therefore use overlapping but different SCM dialects.

### Structural features that SCM expresses well

Standard query syntax expresses these features directly:

- Named node kinds such as `(identifier)`.
- Anonymous grammar tokens such as `"return"`.
- Named and unnamed wildcards.
- Ordered children.
- Field constraints such as `name:`.
- Negated fields such as `!type_parameters`.
- Alternatives in square brackets.
- Grouped pattern sequences.
- Sibling anchors with `.`.
- `?`, `*`, and `+` quantifiers.
- Named captures such as `@name`.
- Multiple top-level patterns in file order.

This model is a good fit for syntax highlighting, tags, folds, injections, and local structural indexing.
One query can produce many matches and many ordered captures.

Landing's parser lowers these forms to structural `Program` instructions.
It resolves all names against the selected Tree-sitter grammar first.
The executor receives numeric kinds, fields, capture slots, and verified control flow.

### Predicate and directive features in Landing

Landing lowers a specific union of provider extensions.
Its current capture predicates include these families:

- Text or capture equality with positive, negative, `all`, and `any` forms.
- Rust regular-expression matching.
- Lua-pattern matching.
- String-set membership and substring tests.
- Captured-node kind tests.
- Parent and ancestor kind tests.
- Same-line tests.

Its current directives include these operations:

- Set match or capture properties.
- Offset a capture span by row and byte column.
- Strip text through a regular expression.
- Select adjacent documentation captures.
- Convert capture text to lowercase ASCII.
- Derive a language property from a MIME type.

These operations are not an embedded general-purpose language.
Each accepted name maps to one typed value or output program.
An unknown predicate normally causes a compilation error.

There is one documented compatibility exception.
Landing validates but ignores `#is?` and `#is-not?` property predicates.
Their exact semantics need catalog-level scope and reference relations.
The design tracks that missing work in NER-1690.
Thus, the current branch does not have complete host-semantic parity for those predicates.

### SCM is not Landing's runtime intermediate representation

The phrase "SCM intermediate language" is easy to misread.
Landing does not execute SCM and does not store it as runtime bytecode.

Its actual compilation path is:

```text
SCM bytes
  -> tokens with source ranges
  -> typed query AST
  -> grammar-resolved ordered query form
  -> Program words and immutable tables
  -> Program verifier
```

Only the verified `Program` reaches an executor.
SCM is an input frontend and a benchmark corpus format.
The typed query AST is temporary compiler data.
The verified `Program` is the durable semantic boundary.

This distinction matters for Rerail.
Rerail can add another frontend that produces `Program` without using SCM syntax.
The packed backends do not need to know which frontend produced the program.

### What SCM cannot express by itself

SCM cannot directly express these Rerail concepts:

- A policy clause, rule mode, path scope, or placement.
- A violation code, anchor, message, fix, or suppression policy.
- Arbitrary ordered `all`, `any`, and `not` predicate graphs.
- Transactional capture rollback across Boolean terms.
- Three-valued unknown results and withheld bindings.
- Reusable utilities with parameters and work limits.
- A relation with neighbor, end, or rule-valued `stopBy` behavior.
- First-related-match capture commitment on every relation axis.
- General ancestor, descendant, and sibling counts with bounds.
- Count values and offsets for message interpolation.
- Rerail comparison modes and semantic text collations.
- Scope, binding, value, resolution, parsed-literal, layout, or call semantics.
- Cross-file projections, joins, require-each clauses, or grouped budgets.

A host can attach custom predicates for some of these operations.
That does not make SCM a complete representation.
It creates host calls whose meaning lives outside the query.
Such calls are incompatible with callback-free Metal execution until they become verified operations.

### Where SCM only approximates Rerail structure

Some Rerail leaves have obvious SCM-shaped equivalents:

| Rerail operation | Possible SCM shape | Main limitation |
| --- | --- | --- |
| `kind` | `(kind_name)` | Direct only for one concrete grammar kind. |
| node `regex` | Capture plus `#match?` | Adds capture and host-predicate semantics. |
| simple `any` | `[...]` | SCM alternatives are structural, not general rule branches. |
| direct `has` | Nested child pattern | Does not preserve every walk and binding rule. |
| field relation | `field: (pattern)` | Covers a structural field edge only. |
| sibling position | Anchors and sequences | Does not cover general `nthChild.ofRule`. |
| capture equality | Two captures plus `#eq?` | Aggregate and missing-capture rules vary by host. |

Ast-grep code patterns are also not SCM patterns.
Both use Tree-sitter trees, but their matching algorithms differ.

Ast-grep patterns include these semantics:

- Language preprocessing before pattern parsing.
- Contextual patterns with a selected internal node.
- `MatchStrictness` rules for named, anonymous, and trivial nodes.
- One-node and multi-node metavariables.
- Repeated metavariable equality.
- Ast-grep-specific ellipsis behavior.

A compiler can produce a large SCM pattern that resembles one ast-grep pattern.
The generated query does not automatically have the same behavior.
A proof needs all strictness, capture, repetition, and ordering cases.
Direct `PatternNode` lowering removes the extra syntax conversion and makes mismatches explicit.

### Why SCM remains useful

SCM was useful on Landing for three historical purposes:

1. It supplied a large and realistic structural-query benchmark.
2. Native Tree-sitter `QueryCursor` supplied an independent oracle for that benchmark.
3. Its features drove a capable structural, value, output, and packed execution core.

That purpose is complete for this port.
Rerail must take the executor components, not the SCM frontend.
The shared point is the verified `Program` with explicit Rerail operations.

## Compatibility analysis

### Compatible representation concepts

Both repositories use a source-revision-bound preorder tape.
Both use a six-byte compact record and a restored navigation table.
Both expose these compatible concepts:

- Stable preorder node ordinals.
- Parent, direct-child, sibling, and descendant navigation.
- Byte spans into immutable UTF-8 source.
- Numeric kinds and fields.
- Named, extra, missing, and error flags.
- Candidate roots as node ordinals.

These concepts make a zero-copy logical adapter possible.
Rerail does not need a second parse or a second tree.

### Tape incompatibilities

The serialized tape formats are not interchangeable.
Both currently claim version 4 and use `TMPTAPE4`.
However, their final header field has a different meaning.

Rerail stores explicit syntax-error ranges for Blade.
Landing stores Tree-sitter supertype entries in the same format position.
A reader from one repository can misinterpret the other repository's bytes.

The first port must not copy Landing's tape decoder.
It can read the restored Rerail `TapeDoc` while version 5 is implemented.
Version 5 then replaces both conflicting version-4 formats with one generic tree contract.

Rerail also hides several transducer accessors behind `cfg(test)`.
The port must make those accessors available to the packed module.
It must not expose them as a public crate API.

Landing records Tree-sitter supertypes.
Tape version 5 generalizes these entries as optional abstract node classes.
Rerail rejects abstract kinds in current policy rules, so product programs do not need class tests initially.

Version 5 also removes Blade's combined overlapping tree.
The Blade template and generated PHP program become separate generic tapes.
A workspace source map maps PHP findings back to Blade bytes.

Kind identifiers are compatible only inside one dialect and grammar build.
Programs must include the grammar identity in their cache key.
A program for TypeScript must never execute against a TSX or Blade tape.

### Compatible execution concepts

The following concepts map well to Landing's runtime:

- One verified program for one grammar-specific rule.
- Candidate roots supplied by Rerail's anchor index.
- Preorder root processing.
- Kind, field, flag, span, and source-text tests.
- Ordered alternatives and captures.
- Bounded work and explicit failures.
- Stable task and local result ordinals.
- One packed call for many programs and roots.

Rerail's `FileAnchorIndex` can remain unchanged.
Its selections contain index ordinals, not guaranteed tape ordinals.
The batch builder must convert each selected node through `node_id()` before it creates a task.
The new runtime does not need Landing's candidate storage schema.

### Incompatible rule semantics

An SCM query is a positive structural pattern language.
A Rerail AST rule is an ordered, stateful predicate graph.
The following Rerail semantics exceed the current Landing `Program`:

- Arbitrary conjunction and negation over nested rules.
- First-match binding behavior in relations.
- Rule-valued `stopBy` boundaries.
- Counts with minimum, maximum, exact-value binding, and offsets.
- Utility calls, parameter frames, and utility work limits.
- Three-valued unknown propagation.
- `violation.each` exclusion and repeated root matching.
- Rerail comparisons and collation modes.
- Resolution, value, binding, parse, layout, and call semantics.
- Violation anchors, message templates, fixes, and suppression.
- Cross-file projections and consumer clauses.

These operations cannot use a host callback from Metal.
Each supported operation needs a verified packed representation.
Unsupported complete clauses must stay on the current matcher route.

### Pattern incompatibilities

Rerail patterns use ast-grep's `PatternNode` and `MatchStrictness`.
Tree-sitter query syntax has different semantics.
A textual conversion to SCM loses information.

Important differences include these items:

- Ast-grep parses code-shaped patterns after language preprocessing.
- Contextual patterns select one node from a larger parsed context.
- Smart strictness treats named and anonymous nodes differently.
- Other strictness modes change kind, text, and trivial-node behavior.
- `$NAME` captures one node and repeated names require equal values.
- `$$$NAME` captures an ordered node sequence.
- Ellipsis matching has ast-grep-specific greedy and trivial-node rules.
- Pattern matching uses ordered children, not only Tree-sitter fields.

The compiler must lower `PatternNode` directly.
It can use Landing's structural instructions and scheduler.
It needs ast-grep-specific lowering rules and parity fixtures.

### Runtime and publication incompatibilities

Landing emits generic matches, captures, and properties.
Rerail emits policy violations and clause failures.
The difference is not only a storage format.

Rerail performs these actions after a structural match:

- Apply transformations.
- Reject matches that overlap syntax errors.
- Derive the configured violation anchor.
- Render messages and fixes.
- Expand `each` results.
- Apply suppression directives.
- Enforce the witness limit after suppression.
- Publish cross-file projections.

The packed path must preserve this order.
A host-only post-filter is unsafe when an earlier output limit can hide later unsuppressed findings.
The executor must either perform the filter or reserve a proven complete output bound.

## Target `Program` design for Rerail

### Program container

The port must evolve Landing's `Program` into a versioned container.
The container must keep separate typed subprograms:

```text
Program
  header
    format version
    grammar identity
    feature bits
    table requirements
    limit model
  MatchProgram
    structural NFA instructions
    pattern and navigation instructions
  RuleProgram
    ordered boolean terms
    relation walks
    utility calls
    comparison and count operations
  ValuePrograms
    exact, prefix, regex NFA, and later semantic value operations
  CapturePrograms
    capture predicates and comparisons
  ReductionPrograms
    count and each state
  OutputProgram
    violation anchor, templates, fixes, suppression, and projections
  immutable tables
    strings, labels, kinds, fields, constants, and call frames
```

The split is conceptual.
The packed ABI can store these tables in contiguous arrays.

The existing structural instruction stream remains useful.
It must not become a general host-call bytecode.
`RuleProgram` coordinates complete Rerail rule behavior around structural subprograms.

### Execution transaction model

Each rule operation must define its environment transaction.
The executor needs explicit instructions for these actions:

- Start an environment checkpoint.
- Commit or discard new bindings.
- Fork from the same input environment.
- Call a subprogram with a shared or isolated frame.
- Mark a value as unknown.
- Withhold bindings after an unknown term.

This model is necessary for `All`, `Any`, `Not`, relations, and utility calls.
Plain NFA branching does not provide these semantics.

Capture storage must preserve these distinct value types:

- One node.
- An ordered list of nodes.
- Transformed bytes.
- A count value for message interpolation.
- An unknown or unbound value.

A capture key alone is insufficient.
The packed ABI must record the capture type and multiplicity.

### Relation and count model

A relation instruction must include these fields:

- Axis: parent, ancestor, child, descendant, next sibling, or previous sibling.
- Walk mode: neighbor or end.
- Optional stop subprogram.
- Optional field gate.
- Candidate subprogram.
- Binding transaction mode.
- Three-valued mode.

The executor must visit candidates in the current matcher order.
It must test the stop rule on the same nodes as the current matcher.
A matching relation commits bindings from the first accepted candidate.

A count instruction must also include minimum, maximum, binding name, and offset.
It must discard bindings from the counted rule.
It must continue past the minimum when an exact bound value is required.
Unknown candidates must produce the same lower and upper count bounds as Rerail.

### Side tables

Some semantic operations need data that is not in `TapeDoc`.
The workspace must construct immutable packed side tables before execution.

Likely side tables include these items:

- Line starts and suppression directives.
- Syntax-error ranges.
- Scope, binding, and resolution tables.
- Module and path components.
- Call summaries and call-graph answers.
- Parsed literal subdocuments.
- Cross-file relation rows.

The program declares each required table.
The batch builder rejects a task when a required table is absent.
The executor must not request data through a callback.

Some side tables require a separate phase.
For example, call summaries and cross-file projections need producer results first.
Rerail must keep its phase scheduler and dispatch a later packed consumer batch.

## Direct lowering of real Rerail rules

### Step 1: Preserve the current frontend

Keep the current JSON and YAML validation.
Keep `compile_ast_rule` as the source of term ordering and grammar gaps.
Keep current capture analysis, soundness analysis, and violation validation.

Add a lowering pass after `GrammarMatcher` construction.
The input is the complete `GrammarMatcher` and `ViolationSpec`.
The output is `Result<VerifiedProgram, UnsupportedReason>`.

Do not lower isolated leaves and combine routes later.
A clause enters the packed route only when all observable behavior lowers.

### Step 2: Lower grammar-specific symbols

Build one program for each `GrammarMatcher` dialect.
Use the same `GrammarDialect` that constructed the matcher and tape.

Resolve and record these values during compilation:

- Kind and field numeric identifiers.
- Named and anonymous node flags.
- Root-kind sets.
- Capture and utility slots.
- Literal byte strings.
- Regex NFAs.
- Violation template segments.

Include the grammar build identity in the program digest.
Reject execution when the tape grammar identity differs.

### Step 3: Lower ast-grep patterns from `PatternNode`

The language adapter already asks Tree-sitter to parse each code-shaped pattern.
The resulting `PatternNode` is the correct compiler input.

Lower each node recursively:

- `Internal` emits a kind test and an ordered child sequence.
- `Terminal` emits strictness-specific kind and text tests.
- `Capture` emits one-node capture and repeated-name equality tests.
- `Dropped` emits a match without retained capture data.
- `Multiple` emits a repeated child-sequence path.
- `MultiCapture` emits the same path with ordered capture retention.

The lowerer must encode every `MatchStrictness` rule.
It must also encode contextual root-kind behavior.
The first implementation can accept only Smart strictness.
It must reject all other modes at compilation.

Do not claim support from shape alone.
Each accepted shape needs parity with ast-grep 0.45.3.

### Step 4: Lower simple matcher terms

Add support in this order:

1. `Kind`, node-text `Regex`, and `Never`.
2. `Pattern` with Smart strictness.
3. `All` without unknown values.
4. `Any` with exact first-branch binding behavior.
5. `Not` with an isolated environment.
6. Constraints on single-node captures.
7. Parent, child, ancestor, descendant, and sibling relations.
8. Field relations and `nthChild`.

`All` must retain compiler term order.
The lowerer must not reorder terms for GPU convenience.
Anchor selection remains a separate optimization.

### Step 5: Lower utilities and reductions

Reject utility cycles as the current compiler does.
Lower each reachable utility once into a subprogram table.

Use shared environments for plain utility references.
Use explicit argument frames for parameterized calls.
Charge one utility step at the same semantic point as the current matcher.

Then add relation counts.
The first production target is the shipped complexity rules.
That target requires these operations:

- Utility references.
- Ordered `any` alternatives.
- Descendant walks with rule-valued `stopBy`.
- Count minimums and exact count bindings.
- Count offsets.
- Message interpolation from the count value.

### Step 6: Lower violation behavior

Extend `OutputProgram` with Rerail operations:

- Witness, start, end, file-start, and capture anchors.
- Single and multi-capture text interpolation.
- Count-value interpolation.
- Fix range and replacement rendering.
- Syntax-error overlap rejection.
- Suppression-table lookup.
- Cross-file field projection.

The output stage must enforce Rerail's order.
Suppression must occur before the witness limit consumes a slot.

`violation.each` needs a reduction over matches from one root.
The reduction key is the selected capture's binding range.
The first unbound match remains valid.
Later duplicate or unbound matches stop enumeration as the current matcher does.

### Step 7: Lower semantic families

Add one semantic family at a time.
Use packed side tables where the operation reads prepared file data.

A practical order is:

1. Text comparison, transforms, and collation.
2. Layout and line-based operations.
3. Arguments and format arguments.
4. Scope, binding, value, and resolution operations.
5. Parsed literals and host tests.
6. Call summaries and call-graph operations.
7. Cross-file projections and consumer predicates.

Each family needs a backend-neutral Rust interpreter implementation first.
Then each packed backend must implement the same verified operation.

### Step 8: Keep host phases outside the executor

The workspace still owns orchestration.
It performs file selection, parsing, tape restoration, and phase scheduling.
It also builds immutable side tables.

The executor owns rule meaning after dispatch.
It cannot call the current matcher for one operation.
It cannot retry a failed task through the current matcher.

Cross-file execution uses separate packed phases:

```text
file-local producer programs
  -> projected rows and call summaries
  -> deterministic workspace merge
  -> packed consumer side tables
  -> consumer programs
  -> violations and failures
```

This design keeps the executor callback-free.
It also preserves Rerail's current dependency order.

## Backend routing

Store a route for each dialect-specific clause:

```text
Current
Packed {
  program_digest,
  minimum_format,
  required_features,
  permitted_backends
}
```

The compiler selects the route.
The workspace selects one permitted backend for the complete packed batch.

A clause cannot change routes during execution.
A task failure cannot trigger current-matcher execution.
Capacity and work failures are clause failures.
Backend initialization failure can prevent packed admission before execution starts.

During migration, Metal-capable macOS uses this preference order:

1. Metal JIT.
2. Metal interpreter for diagnostics and parity runs.
3. The current matcher for clauses compiled with `Current`.

Linux, Windows, and unsupported Macs use `Current` initially.
The later CPU JIT becomes their packed backend.
The Rust interpreter remains a parity tool and a debug backend.

## Parity strategy

### Oracle levels

Use three oracle levels:

1. `matcher::run_clause` is the policy-semantic oracle.
2. The packed Rust interpreter is the packed-ABI oracle.
3. Metal and CPU JIT results must equal the packed Rust results.

Compare complete clause outcomes, not only matched spans.
Each comparison includes these values:

- Success or failure classification.
- Root, match, and violation order.
- Multiplicity.
- Capture names, types, nodes, ranges, texts, and order.
- Anchors, messages, fixes, and projections.
- Suppression behavior.
- Syntax-error behavior.
- Work, witness, and capacity failures.

### Differential fixture matrix

Generate fixtures across these dimensions:

- Every grammar dialect used by repository rules.
- Named, anonymous, extra, missing, and error nodes.
- Every pattern strictness and metavariable form.
- Repeated captures and multi-captures.
- Empty, first, middle, and last sibling positions.
- Each relation axis and `stopBy` mode.
- Binding success, failure, rollback, and unknown results.
- Counts below, inside, and above their bounds.
- Suppressed and unsuppressed matches near witness limits.
- Corrupt packed ranges and every arena exhaustion point.

Also compile every repository policy.
The report must show the first unsupported operation for each current-route clause.
It must group failures by operation family and rule pack.

### Shadow execution

Add an opt-in shadow mode before production routing.
The workspace runs both paths from the same `TapeDoc` and selected roots.
It compares results before publication.

Shadow mode must not alter production output.
It must record a compact mismatch artifact with these values:

- Policy, clause, dialect, and program digests.
- Source digest and candidate roots.
- Current and packed outcomes.
- Backend name and packed format version.

Do not send source text in normal telemetry.
Keep local source only in explicit developer artifacts.

## Required implementation sequence

1. Freeze the semantic inventory and output contract.
2. Add production transducer accessors to Rerail `TapeDoc`.
3. Port the packed ABI and Rust interpreter.
4. Add the versioned `Program` verifier.
5. Lower kinds, text, and Smart terminal patterns.
6. Lower full Smart structural patterns from `PatternNode`.
7. Lower ordered boolean terms and relations.
8. Lower utilities, counts, and complexity-rule output.
9. Port the Metal interpreter, Metal JIT, and workspace pool.
10. Connect one packed batch to Rerail's file evaluation path.
11. Add transforms, `each`, fixes, and suppression.
12. Add semantic side tables and semantic operation families.
13. Add packed producer and consumer phases for cross-file rules.
14. Port the callback-free CPU JIT.
15. Remove current routes only after the inventory reaches full coverage.

Each step has two gates.
The Rust interpreter must equal the current matcher.
Each accelerated backend must equal the Rust interpreter.

## Designs to reject

Do not translate Rerail rules to textual SCM.
The translation cannot preserve ast-grep pattern and policy semantics.

Do not treat Tree-sitter's native query cursor as the production executor.
It cannot execute Rerail's complete rule model.

Do not run unsupported predicates as Rust callbacks from Metal or JIT code.
Callbacks break the packed boundary and backend equivalence.

Do not split one clause between packed and current execution.
Bindings, order, unknown values, counts, and limits cross operation boundaries.

Do not assume that matching spans prove parity.
Violation rendering and failure behavior are part of the contract.

Do not read Landing tape bytes with Rerail's decoder.
The two version-4 formats assign different meanings to one header field.

Do not port Landing's normalized result schema with the executor.
Rerail's bounded violations and existing store have a different workload.

## Completion criteria

The port is complete when all of these statements are true:

- Every shipped Rerail AST clause produces a verified `Program` for each applicable dialect.
- All packed backends produce identical packed outcomes.
- Packed and current paths produce identical Rerail outcomes on the parity corpus.
- Work and capacity failures never publish partial results.
- No executor parses policy syntax, SCM, regex source, Lua source, or ast-grep syntax.
- No executor calls a host matcher operation.
- Mixed current and packed clauses share one parsed source and one restored tape.
- Metal-capable macOS uses one packed invocation for each admitted file batch.
- Other supported platforms retain correct execution until the CPU JIT is available.
- Existing Rerail persistence remains compatible unless a measured migration replaces it.
