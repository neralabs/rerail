# Batched matcher fragments

## Decision, 2026-10-25

The generic SQL state machine is a rejected compiler architecture.
SQL must compose typed relations with joins, exclusions, aggregation, and structural closure.
Coherent matching operations remain inside batched native matcher fragments.
The compiler must not convert ordinary composition into program counters, call stacks, or continuation rows.

This decision supersedes the shared-function scheduler in the earlier transition plan.
The qualification harness, native oracle, and recorded performance ceilings remain authoritative evidence.
Old coverage reports describe the old executable. They do not qualify this replacement.

## Evidence and scope

Before commit `049c5873`, eleven catalog grammars remained refused.
Five exceeded compiler operator limits, including B615 and SIM905.
The exact clause names are `b615`, `duplicate-isinstance-call`, `enumerate-for-loop`, `multiple-starts-ends-with`, and `split-static-string`.
Six lacked recursive context contracts: B020, F405, `is-literal`, and the three forward-annotation clauses.
These were SQL compiler limitations. The native matcher already executes these constructs.

The wrong decision introduced shared instructions and return continuations to solve expansion and semantic problems together.
The replacement separates those problems. Relational factoring addresses duplication. Matcher fragments own sequential semantics.

## Existing capabilities

`AstMatcher` supports patterns, ordered alternatives, captures, searches, recursive utilities, parameterized calls, and semantic predicates.
`run_clause` preserves root order and utility allowances within one file.
`enumerate` implements repeated capture enumeration through exclusion and rematching.
The pattern virtual table already accepts a complete input set through SQLite's IN constraint interface.
Its cursor advances over completed output. It does not ask SQL to execute each matcher instruction.

The integration worktree has a typed producer registry with ordering, multiplicity, identity, and completion contracts.
The spike must inform that interface. It does not introduce another production registry.

## Fragment contract

A compiled fragment retains its matcher, utility definitions, capture schema, and context requirements.
Demand contains a request identity, document identity, root node, and required input captures.
Output contains completion, an explicit outcome, match order, and typed exported captures.
False, unknown, incomplete, and failed outcomes remain distinct.
Lists and parsed spans retain document identity and native capture semantics.

One filter invocation consumes an operator's whole demand batch.
Native code evaluates roots within each file and context. SQLite receives completed output only.
Native code never asks SQL to evaluate a child matcher or utility.
File failure discards provisional output for that file, without discarding successful files.

Fragment boundaries follow semantics, not individual terminal nodes.
Keep capture rollback, recursive utility calls, and ordered searches inside a fragment when they share state.
SQL composes independently useful match relations. A whole-rule native control alone does not prove this composition.

## Open semantics

Exact work charges, cache-fill accounting, depth peaks, and clause-wide memo history can constrain fragment boundaries.
The initial native binding preserves them within a complete execution region.
Splitting that region requires an explicit session contract or a separate semantic decision.
The spike must not claim exact accounting across independently executed fragments.

Each logical contract needs a pure-SQL binding under the agreed architecture.
The native spike does not establish that binding for every fragment.
Unresolved SQL bindings remain explicit. A generic SQL interpreter is not an acceptable implementation.

## Planning and measurement

Production planning reads kind and edge statistics from SQL-held indexes or aggregate tables.
Compiled output uses a document digest and compiler version, with recorded statistics dependencies.
Preparation and compilation are separate from warm execution.
Sharing requires compatible semantic inputs and candidate scope, with measured reuse.

Acceptance requires native parity, boundary invocation counts, intermediate cardinalities, and CPU and wall time.
VM counts remain diagnostics. Native matcher work must appear in timing even when SQLite executes few instructions.
Full repository and per-edit comparisons remain required before production adoption.

## Journal

### Start

Read the matcher, terminal batching, catalog refusal evidence, and integration producer contracts.
Preserved unfinished interpreter optimizations in `/tmp/rerail-before-fragment-transition/` before removal.
The spike starts from this branch. It is not a production cutover or a completed rebase onto the integration worktree.
The integration worktree contains substantial staged work and remains untouched.

The implementation will remove generic scheduler emitters, then test batched fragments against all eleven original refusals.
Independent composition and capture-bearing demand controls will distinguish a fragment interface from a whole-rule wrapper.

### First boundary experiment

Removed the generic finite, captured-recursion, parsed-recursion, and pure instruction-loop SQL emitters.
Removed the scheduler-only search, binding, and projection adapters.
Historical control IR remains for diagnostics. Its emission entry points return explicit refusals.
The compiler no longer selects shared-function interpreter alternatives after a size refusal.

Added a native fragment table with one batch filter, typed captures, explicit unknown outcomes, and file-isolated failure publication.
The composition control passes: SQL joins two fragment relations, excludes matches, and aggregates their outputs.
The first B615 run preserves native output but fails the work-accounting comparison.
The experiment reused scope caches from the preceding native run, which invalidated the accounting and timing comparison.
The corrected harness starts both executions with fresh scope caches.

The native baseline now groups request keys once, rather than scanning every request again for each file.
That correction prevents benchmark overhead from inflating the native time.
False results complete their demand without producing SQL rows. Unknown and failed outcomes remain explicit.
The enumeration control uses native exclusion and rematching inside one batch, with an explicit output ordinal.
Enumeration with prebound capture inputs remains refused until its exclusion contract is defined.

### Qualified fragment boundary

The corrected run passes five focused tests, including all eleven original refusals across 597 fixture files.
Every grammar preserves native matches, exported captures, and remaining work within its file execution region.
The run covers 196,678 candidate roots and returns 226 matches.
Each grammar enters its native table once. No matcher instruction crosses back into SQL.

The sum of per-grammar median execution CPU is 232.1 ms native and 240.5 ms through SQL.
SIM905 takes 43.6 ms native and 44.0 ms through SQL, with 1,245 SQLite VM instructions.
These figures use three paired executions in an opt-level-1 test build.
They exclude common tape setup and preparation. They are not production or full-repository performance claims.

The next measurement runs all eleven grammars as one job over the same files.
It includes fresh tape parsing, candidate discovery, batch setup, preparation, result assembly, and teardown.
The native control processes files and rules directly, without constructing SQL node facts.
The SQL job preserves separate candidate scopes and combines only final outputs in one statement.
Policy compilation is reported separately.

The spike does not reopen the old pure-SQL catalog qualification gate.
The native fragment binding is explicit in the evidence, and no production selector or opt-in flag is added.

### Combined job result

The final run passes all five focused tests, including the combined eleven-grammar job.
The job covers all 597 fixture files and preserves 226 native matching results.
Each native fragment receives one batch. The job executes one matching SQL statement.
Final publication uses 1,625 SQL bytes and 11,873 SQLite VM instructions.
The producers keep separate candidate scopes and narrow schemas. Only final publication has a common union schema.

Three alternating-order cold runs give these medians:

| Measurement | Native | Native fragments through SQL |
| --- | ---: | ---: |
| Complete job CPU | 288.2 ms | 287.5 ms |
| Complete job wall time | 828.9 ms | 761.9 ms |
| Batch setup CPU, including parsing | Not separately instrumented | 84.7 ms |
| SQL preparation CPU | Not applicable | 0.175 ms |
| Matching and result decoding CPU | Not separately instrumented | 200.6 ms |

The CPU difference does not establish a speed advantage. The spike runs at approximately native cost on this workload.
Wall time includes shared-host scheduling noise. VM counts describe SQLite work only, not native matcher work.
The complete job includes parsing, candidate discovery, setup, preparation, matching, result decoding, and teardown.
Policy compilation is separate and recorded per policy.

The [evidence](../benchmark/sql-matcher-fragment-spike.json) includes every run, boundary counters, EXPLAIN output, fixture hashes, and source hashes.
The [combined SQL](../benchmark/sql-matcher-fragment-spike.sql) shows the complete publication statement.

### Remaining transition work

This is a native-binding spike, not a restored pure-SQL compiler qualification or a production cutover.
The ordinary compiler retains explicit refusals for cases that required the removed interpreters.
Legacy scheduler-specific tests no longer describe a supported emitter. Their old ceilings and evidence remain unchanged.
The focused checks retain native result, capture, work, unknown, enumeration, and failure controls for the new boundary.

The eleven-case test uses complete AST matching regions. A separate control proves SQL joins, exclusion, and aggregation across independent fragments.
Automatic fragment partitioning and production integration with the shared producer registry remain separate work.
Capture-bearing requests enter through immutable batch data. General SQL-produced capture demands still need a relation-handle contract.
The pure-SQL bindings, cross-fragment accounting, full-repository CPU and RSS, and per-edit latency remain unqualified.
The integration worktree and its staged changes remain untouched.

### Reproduction

Build the test executable with the recorded profile and allocator settings.
Run this focused test filter:

```sh
RUST_MIN_STACK=67108864 \
RERAIL_FRAGMENT_EVIDENCE=/tmp/sql-matcher-fragment-spike.json \
  /path/to/test-executable fragment_spike_ --nocapture --test-threads=1
```

The test refuses missing positive witnesses, output differences, work differences, repeated batch invocation, or extra matching statements.
