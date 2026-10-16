# NER-1562 packed executor landing pad

## Purpose

This document defines how Rerail will receive the advanced executor work from the NER-1562 Landing branch.
It also records the local source refs that protect that work.

The landing pad is not a wholesale merge from `nera-landing`.
It is a controlled port into the SQL rerailer architecture.

## Preserved source refs

The Landing repository now has these local archive branches:

```text
archive/ner-1562-packed-executor-2026-10-16
  -> 2f22bcb134ac60ddc468936a89ec2fe8b633cbda

archive/ner-1562-sql-engine-donor
  -> 30a03c642cdd993bec39357c27eb432b28043136

archive/ner-1562-packed-executor-wip-2026-10-16
  -> 780c6cbd7542ec0ad9d13ea8408b535901dcfa7e
```

The first ref protects the 198 committed changes after the published NER-1562 branch.
The second ref protects the final historical SQL engine before its deletion.
The third ref snapshots the two uncommitted profiling files without changing the active Landing worktree.

The packed branch and the completed SQL-engine line are parallel histories.
Their merge base is `b59df7b686b655387b76ed0ac256ea57f179aeab`.
The packed branch is not a descendant of the SQL deletion commit.
Its `transducer/sql_backend.rs` is the terminal virtual table, not the complete historical SQL compiler.

The Landing worktree also contains uncommitted profiling work:

```text
modified: tempo/native/src/policy_dsl/transducer/metal_program_backend.rs
untracked: tempo/docs/cpython-335-seconds-workplan.md
```

Those files are part of the WIP archive branch only.
They remain uncommitted on the active Landing branch.
They must remain in the Landing worktree until that session commits or discards them.
Rerail work must not modify or clean that worktree.

## Rerail branches

The implementation uses a stacked branch structure:

```text
origin/main
  -> jbarnes/ner-1562-sql-rerailer
       -> jbarnes/ner-1562-packed-landing-pad
```

The SQL branch owns these items:

- Relational plan contracts.
- Typed staging relations.
- Completion and canonical outcome contracts.
- Historical SQL test recovery.
- Current-matcher differential execution.

The packed landing-pad branch owns these items:

- Packed ABI records and verification.
- The backend-neutral Rust interpreter.
- Packed terminal relation output.
- Metal interpreter and Metal JIT.
- Callback-free CPU JIT.
- Reusable workspaces and sparse rule-file grids.

The packed branch must not introduce another rule-composition engine.
It emits typed relations consumed by the SQL branch.

## Why not merge or cherry-pick the complete Landing branch

The Landing branch starts from a Tempo line that diverged before Rerail existed.
It contains 198 local commits and broad changes to these areas:

- SCM parsing and lowering.
- Packed ABI versions 3 and 4.
- Metal and CPU JIT execution.
- Workspace scheduling.
- Result normalization and SQLite publication.
- Tempo-specific storage and policy names.

Rerail has changed the same areas:

- The public code-rule contract.
- Language adapters and grammar selection.
- Blade parsing and generated PHP.
- Tape version 4 extensions.
- Workspace persistence.
- Rule packs and suppression behavior.

A complete merge or long cherry-pick series will hide semantic conflicts.
It will also import SCM as a product compiler and import Landing's result schema without measurement.

Each port commit must have one responsibility and one differential gate.

## Source eras

The Landing history contains several implementation eras.
They must not be mixed without an explicit conversion.

### Program and SCM era

Representative range:

```text
72c975017 .. 78960ef41
```

This range adds repository indexing, SCM parsing, structural programs, predicates, and output directives.

Use from this era:

- Program operation semantics.
- Structural and predicate test fixtures.
- Typed output concepts.

Do not port:

- SCM as a production input language.
- Provider-specific query compilation.
- Tree-sitter query property behavior as Rerail policy semantics.

SCM remains a test frontend and oracle corpus.
Rerail lowers `GrammarMatcher` and ast-grep `PatternNode` directly.

### Packed ABI v3 era

Representative range:

```text
60536c74a .. 4dff797b4
```

This range contains the coherent executor used by the earlier port analysis.
It includes packed batches, scheduling, Metal execution, CPU JIT work, bounded arenas, and typed insertion.

Use this era as the easiest executable reference.
Its internal records are coherent and its behavior has broad tests.

Do not expose ABI v3 as a persisted Rerail contract.

### Packed ABI v4 era

Representative range:

```text
8ec143a54 .. ecb014feb
```

This range replaces disjoint arenas with shared evaluation arenas.
It includes atomic reservation and generation-safe workspace changes.

Use from this era:

- Final arena ownership rules.
- Capacity checks.
- Completed CPU and Metal record layouts.
- Tests that detect mixed-generation offsets.

Do not port a partial cutover.
The minimum donor point for ABI-v4 code is `ecb014feb`.

### Direct publication and grid era

Representative range:

```text
352e01ed3 .. 2f22bcb13
```

This range adds packed result views, direct normalized publication, parallel Metal roots, query admission, sparse grids, and bounded pages.

Use from this era:

- Sparse file-program grid scheduling.
- Stable logical ordinals.
- Command partition rules.
- Result-page validation.
- Reusable Metal input metadata.

Reassess before porting:

- Normalized SQLite result relations.
- Keyset paging schema.
- Direct publication sinks.
- CPython output cardinality assumptions.

Rerail's SQL rule engine will own high-level composition.
Terminal rows can have materially lower volume than Landing's final normalized result rows.

## Destination contracts

The SQL branch defines one terminal boundary before backend code moves.
The conceptual interface is:

```text
TerminalRequest {
  batch_identity
  grammar_identity
  programs
  files
  candidate_roots
  limits
}

TerminalRelation {
  status_rows
  match_rows
  capture_rows
  property_rows
  transformed_text
}
```

Every range uses checked offsets and lengths.
Every requested `(file, program)` pair has one status outcome.
A failed evaluation contributes no partial match rows.

The SQL engine can consume the relation through either:

- Sorted typed temporary-table insertion.
- A read-only batch virtual table over immutable arrays.

The staging spike selects the physical boundary.
Backend code cannot depend on one choice before that gate closes.

## Port map

| Landing source | Rerail destination | Treatment |
| --- | --- | --- |
| `transducer.rs` program model | `code_rule_dsl/packed/` | Port operations after direct Rerail lowering exists |
| `packed_batch.rs` | `code_rule_dsl/packed/batch.rs` | Port typed ranges and validation |
| `scheduler.rs` | `code_rule_dsl/packed/scheduler.rs` | Port stable work and failure semantics |
| `packed_arena.rs` | `code_rule_dsl/packed/arena.rs` | Port only from completed ABI-v4 donor |
| `metal_batch_abi.rs` | `code_rule_dsl/packed/metal/abi.rs` | Port with layout tests |
| `metal_program_backend.rs` | `code_rule_dsl/packed/metal/` | Port after terminal relation freezes |
| `native_ir.rs` and `native_cpu*` | `code_rule_dsl/packed/cpu_jit/` | Port after Rust and Metal parity |
| `output.rs` | `code_rule_dsl/packed/relation.rs` | Convert output to terminal relation rows |
| `sql_backend.rs` | SQL staging spike | Use as a test reference, not production shape |
| `scm.rs` | test support only | Do not expose in product code |
| normalized content-store rows | no initial port | Measure Rerail publication first |
| workspace integration | no direct port | Rebuild around Rerail batches and SQL plans |

## Direct lowering requirement

A Rerail clause compiles in two coordinated parts:

1. The SQL compiler lowers high-level rule composition.
2. The terminal compiler lowers structural `PatternNode` operations and bulk fact requests.

The terminal compiler must preserve:

- Smart strictness.
- Contextual root behavior.
- Named and anonymous terminal rules.
- Repeated capture equality.
- Multi-capture ellipses.
- Capture and alternative order.
- Exact grammar identity.

SCM can produce test Programs for parity.
It cannot become the Rerail compiler boundary.

## Tape boundary

The first packed port reads restored Rerail `TapeDoc` values.
It must not read Landing tape bytes.

Both repositories call incompatible persisted layouts `TMPTAPE4`.
Rerail uses the final table for Blade syntax-error ranges.
Landing uses it for supertype entries.

Generic tape version 5 will later provide:

- One source.
- One grammar identity.
- One rooted tree.
- Generic error and missing nodes.
- Optional abstract node classes.

Blade template syntax and generated PHP remain separate analysis documents.
Their map belongs to the workspace.

## Commit structure

The packed landing pad will use small reviewable commits in this order:

1. Add source provenance and the empty module boundary.
2. Add ABI scalar types, checked ranges, and layout tests.
3. Add terminal relation records and canonical ordering.
4. Add the Program verifier.
5. Add the backend-neutral Rust interpreter.
6. Add direct Smart `PatternNode` lowering.
7. Add packed batch construction from Rerail tapes and candidate roots.
8. Add Metal interpreter support.
9. Add Metal JIT support and workspace reuse.
10. Add sparse file-program grid scheduling.
11. Add the callback-free CPU JIT.
12. Remove test adapters after direct relation parity passes.

Each commit must build on all release targets.
Metal files stay behind a macOS feature and target gate.

## Test gates

Each backend must pass the same terminal relation fixtures:

- Empty and failed evaluations.
- Named, anonymous, trivia, error, and missing nodes.
- Every sibling position.
- Repeated captures and multi-captures.
- Regex and Lua value programs.
- Capacity boundaries and corrupt ranges.
- Reversed preparation and physical lane order.
- Mixed files, grammars, and program identities.
- Syntax recovery and UTF-8 boundaries.

The Rust interpreter is the packed semantic oracle.
The current matcher remains the complete rule oracle.
The SQL plan comparison remains the final product oracle.

## Merge gates

The packed branch can merge into the SQL branch when:

1. Every packed record has one versioned meaning.
2. The verifier rejects every corrupt range fixture.
3. Rust terminal relations equal current matcher terminals.
4. Metal terminal relations equal Rust.
5. The CPU JIT terminal relations equal Rust.
6. Backend failures return status rows without partial matches.
7. SQL plans produce the same canonical clause outcomes with every backend.
8. No product code parses SCM.
9. No packed executor calls a host matcher.
10. The workspace builds each tape and immutable Program table once per batch.

## First implementation change

The first code change on the landing-pad branch will not copy executor code.
It will define and test `TerminalRelation` in Rerail.

This order prevents the port from inheriting Landing's final publication schema by accident.
It also gives the SQL staging spike and all packed backends one stable boundary.
