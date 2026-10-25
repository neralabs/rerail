# Data-layer rule shapes: measured costs and fixes

Date: 2026-10-25. Engine: the merged-tree release addon. Samples: 300 random files from the 9,021-file data-layer recording, plus the full corpus for single checks only. Every run was capped at five minutes. The "before" numbers are the recorded phase A runs; they were not re-run.

## 1. Why the AST rewrites looked slower in single-rule runs

Phase A rewrote text-regex clauses as syntax clauses (backend-type-strings, hasattr-async and others). In a run of one rule on the full corpus, CPU went from 1–4 s to 30–50 s. On a 1,500-file sample, backend-type-strings went from 2.0 s to 9.2 s of CPU, about half of it system time.

A stack sample of the rewritten run shows where the time goes. The waiting threads aside, the active stacks are dominated by `pwrite`, `fsync` and WAL checkpoints under `anchors::sql::corpus::Corpus::put`. Tree-sitter parsing is about 5% of active samples.

The cause is that every file a syntax clause may match is first published into a temporary SQLite positional index. Candidate roots are then selected from postings (`AnchorPlan::needs_index`). Text-only rules never publish, so a rule moving from text to syntax pays for publication alone when it runs by itself.

**All 60 rules together, 300 files:**

| Policies | CPU | Peak RSS | Findings |
|---|---|---|---|
| Before phase A | 46.3 s user + 4.4 s sys | 1.75 GB | baseline |
| After phase A | 46.7 s user + 4.6 s sys | 1.57 GB | identical set, same 22 failures |

When the 60 rules run together, other syntax rules already publish every file, so the rewrite is cost-neutral. Setting `synchronous=OFF` on the temporary store (the file is unlinked with the store) removed the fsyncs but changed little: system time on the 1,500-file case went from 4.0 s to 3.5 s.

The structural fix is the spec direction. Local rules execute over resident tapes (§20) as transducer programs, and only facts and relational candidates reach SQL. Per-file publication of every node into a positional index should not be on the path of local rules.

## Summary: all 60 checks, 300-file sample

| State | CPU (user + sys) | Peak RSS | Failures | Findings |
|---|---|---|---|---|
| Before phase A (recorded) | 50.7 s | 1.75 GB | 22 | baseline |
| Phase A + shape fixes (§3, §5–§8) | 32.9 s | 1.03 GB | 10 | identical set |
| + engine fixes (§9) | **29.7 s (−41%)** | 1.10 GB | **0** | identical set |

## 2. Where the CPU goes: one rule at a time, 300 files

The floor for any rule run alone is about 1.7 s: process start, compilation, and publication for syntax rules.

| Check | CPU (s) |
|---|---|
| legacy_execution_surfaces | 15.43 |
| serverplan_quality | 11.62 |
| serverplan_state_tripwires | 5.89 |
| internal_dead_code | 4.77 |
| serverplan_dispatch_architecture | 4.66 |
| plan_scope_ownership | 4.65 |
| execution_authority_bypass | 4.43 |
| serverplan_grammar_surface | 4.17 |
| public_api_triples | 3.69 |
| conn_or_txn_union | 3.59 |
| imperative_private_functions … (48 more) | 2.8 down to 1.7 |

Splitting the all-60 run into the 8 M1 "heavy" checks and the other 52 gives 23.7 s and 31.0 s of CPU respectively. The cost is therefore not confined to the M1 set. Every check above about 3 s is a relational clause fed by whole-tree copies.

## 3. Worked fix: legacy_execution_surfaces

**Before.** One relational clause with 25 inputs (`syntax_0` … `syntax_24`) exported every identifier (sharded by first letter), string, call, attribute, argument list, statement, block and literal of every in-scope file. Twenty-three SQL stages then rebuilt `nodes`, `edges`, recursive `ownership`, `receiver_tokens` and the call graph. The final query selects about 15 issue codes, most of them local. The oracle reports zero findings for this check on the corpus.

**After** (`scripts/dsl/data_layer_six/legacy.py`), lowering rule L5:

- **Local AST clauses** cover LEG-020/030/040/050/060/080/081/082/084. They are call, definition, import and if-statement shapes with literal name sets, `filePath` scopes and allowlists, and two further tests:
  - *Receiver token.* The receiver chain is a `has` whose `stopBy` halts at anything other than attribute, call, subscript value or parenthesized expression. A cast's second argument is its own alternative.
  - *Not inside a legacy-decorated function.* An `inside` test.
- **One relational clause, for the joins only**, covers LEG-010 (file boundary status), LEG-021 (a wrapper's other calls), LEG-083 (eager-call reachability: a fixpoint over the in-file call graph) and LEG-085 (import aliases). Its inputs are narrow rows: `fns`, `legacy_fns`, `wrapper_fns`, `classes`, `ident_calls`, `attr_calls`, `other_calls`, `from_imports`, `direct_imports`. The innermost enclosing function comes from span containment, with no tree walk.

| | Before | After |
|---|---|---|
| Fixture parity (80 oracle rows) | exact | exact (setFP 0, setFN 0, failures 0) |
| 300-file sample, CPU | 15.4 s | 1.8 s (the single-rule floor) |
| Full corpus, CPU | > 1,290 s (killed unfinished) | 34 s; 0 expected, 0 actual, 0 failures |
| Relational inputs | 25 whole-tree shards | 9 narrow candidate sets |

The receiver-token relation and the in-file call graph used here (`resolved_calls`, `eager_reach`) recur in other data-layer rules. Under decision 0005 they are candidates for catalog facts (`py.receiver_tokens`, `py.call_edge_local`), defined once and shared instead of re-derived per rule.

## 4. Findings recorded on the way

- The new hasattr-async gate finding (`l2070_batching/execution_runtime.py:5978`) is genuine. It is `getattr(self._backend, "…_async", None)` split across lines, which the oracle's line regex cannot see.
- The rule-level trigger gate (`inside: module` + regex) is evaluated once per file thanks to relation memoization. Literal sets inside a rule's regexes also give a per-file `memmem` prefilter (`GrammarMatcher::admits_source`). Literal-name clauses are therefore close to free on files that lack the names.
- `filePath` needs at least one of `regex`/`notRegex`. An empty allowlist must emit no gate.

## 5. Plain syntax rules: prefilter and recursion costs

plan_scope_ownership, transaction_callback_wrappers, conn_or_txn_union and serverplan_grammar_surface have no relational clause, so L5 doesn't apply to them. Two other causes accounted for their excess cost:

- **`value` tests disable the per-file literal prefilter.** A `{'value': {'regex': …}}` string test makes the clause three-valued, and the compiler then drops the `memmem` prefilter (`reads_values`). Testing the literal's quoted source text (a `regex` on the string node) keeps the prefilter and finds the same matches, apart from names spelled through implicit concatenation or escapes.
- **Self-recursive helpers (`has: {matches: self}`) roughly double a clause's cost** even when few nodes reach them. A plain `has … stopBy: end` search, or a helper unrolled to a fixed depth that falls back to the recursive form, matches exactly at the floor's cost.
- **Cheap tests go first.** Each clause now runs the cheap tests before the deep ones: text conditions on the candidate (`CtePublicationContract`, `Callable` …) come before tree walks, and allowlist exclusions come last.

| Check | Fixture parity | 300-file CPU before → after | Cold-phase user ms (floor) |
|---|---|---|---|
| plan_scope_ownership | exact (30 rows) | 4.65 → 3.13 | 2178 → 1040 (1058) |
| transaction_callback_wrappers | exact (20 rows) | 2.54 → 1.73 | 946 → 533 (407) |
| conn_or_txn_union | exact (15 rows) | 3.59 → 2.49 | 1734 → 720 (721) |
| serverplan_grammar_surface | unchanged | already at floor: its scope is `**/*.py`, so the cost is parsing | |

Implication for the engine: the prefilter and the transducer's `TEXT_ID` literal tests (spec-transducer §5.2) must survive three-valued value tests. Recursion through `matches` should compile to a bounded walk rather than a re-entrant match. Both are now tracked against M3.

## 6. Batch six: state, dispatch, authority

| Check | Fixture parity | 300-file CPU before → after |
|---|---|---|
| serverplan_dispatch_architecture | exact (59 rows) | 4.66 → 1.19 |
| execution_authority_bypass | exact (5 rows) | 4.43 → 3.94 (scope floor ≈ 3.3) |
| serverplan_state_tripwires | exact (79 rows) | 5.89 → 5.33 |

- **Gate order matters.** The whole-file `syntax_gate` inputs tested their source gates, which walk up to the module, before `kind: module`, so every node of every file walked to the root. Testing kind first, and putting each input's own shape before its gates, removed that cost.
- **Path-only inputs get no prefilter.** A `filePath` check doesn't skip node-by-node matching, so dispatch's 25 inputs were checked in every file even though only `base_server_plan.py` can match. A module-text requirement (`class BaseServerPlan`) gives every input the per-file `memmem` prefilter. The engine should treat a constant-false `filePath` as a file prefilter, the way it treats literals; that belongs to the residency/scope work of §20.
- **A `value` predicate costs time per file even where the prefilter should skip the file.** serverplan-ddl costs about 0.45 s on a sample with no `ddl_effect_lazy`/`grammar_effect_lazy`; removing the `value` test brings it to the floor. This confirms §5.
- **Remaining:** the state-flow taint relation is still a whole-tree copy. Its `direct_sinks` stage counts every ancestor of each sink, about 1.1 s over the floor. It is the next L5 rewrite and the natural consumer of a `py.local_flow` catalog fact (decision 0005).

## 7. serverplan_quality

The 54-stage contract clause was not the main cost. Removing one clause at a time gave the split: the project-wide `quality-conditional-1` cost about 5.8 s of the 11.6 s, because it copied tree nodes from every file. `quality-contracts-1` cost about 2 s, `-2` and `-3` together about 1 s, and the taxonomy clauses about 0.5 s.

- **`conditional.py`, rewritten as one global clause, `conditional-architecture`.** It has the same detectors, locations, key and code (SPQ-029). It reads about 17 narrow inputs (classes, base lists, import names, functions, `self.x` stores, call shapes, validation-method blocks) and uses span containment instead of nodes/edges/ownership/descendants. Its remaining 2.4 s is all AST input matching; the SQL is negligible.
- **SPQ-051/052 are forbid clauses, and SPQ-053 is a small relational clause** (`ordered-semantic-inventory`). That removes `quality-contracts-2` and `-3`.
- The shadowing test in `test/scripts/test_shared_sql.py` was ported to the new span-based stages (`cclasses`, `cmethods`, `cfunctions`) with the same expectations.

| | Before | After |
|---|---|---|
| Fixture parity (200 rows) | exact | exact |
| 300-file CPU | 11.62 s | 8.18 s |

**Still to do:** `quality-contracts-1`, with about 80 detector rows on whole-file syntax relations. Most of its rows map to forbid clauses. The rest need narrow relational inputs:
- SPQ-001 and SPQ-006: duplicate methods by name;
- SPQ-020: pre-op functions, a fixpoint over names;
- SPQ-030/031: call paths;
- SPQ-042: module string constants;
- SPQ-050: import and scope resolution.

## 8. Batch five: cross-file rules

| Check | Fixture parity | 300-file CPU before → after |
|---|---|---|
| internal_dead_code | exact (43 rows) | 4.77 → 3.02 |
| public_api_triples | exact (38 rows) | 3.69 → 2.56 |
| imperative_private_functions | unchanged | 2.83 (about 650 ms of it is the engine's reference call graph) |
| async_sync_delegation | unchanged | 2.80 (spread across clauses) |

These rules did not copy the tree. Their cost came from inputs the engine could not narrow to root kinds:
- `identifier` inside an `any` with other kinds;
- an `inside` whose target is a `not` or contains a nested relation.

In both cases every identifier in every file was a candidate. Splitting each input by the kind of place it reads, and adding a redundant but narrowing `inside class_definition (field superclasses)` condition, fixed it. The engine lesson for M3: root-kind inference must see through `any` of kinds and through negated or nested `inside` targets. A transducer program gets this from its `KIND_SET` entry test.

## 9. Engine fixes found on the way

- **The prefilter is kept for three-valued matchers** (`native/src/code_rule_dsl/compile/mod.rs`). `RequiredText::of` reads only two-valued source-text tests (node regexes, pattern leaves), never `value` tests. It reads them through connectives that are monotone under Kleene logic: `all`, `any`, `has`/`inside`, double `not`. A file missing a required literal therefore fails the positive match outright, and skipping it is sound even when the rule can be unknown elsewhere. The compiler used to drop the prefilter for any rule that read values. Test: `matcher::tests::three_valued::a_three_valued_rule_keeps_its_source_prefilter`.
- **Stage index names are unique per physical table** (`native/src/code_rule_dsl/relational.rs`). A stage that defers columns stores rows in `__stage_<name>` behind a view `<name>`; one that defers none stores rows in `<name>`. Both named their indexes `__<name>_<n>`, and SQLite index names are global. When two clauses shared a stage name (`functions`, `calls`, `classes` …) but differed in deferral, declaring the second table dropped only the view, and the second `CREATE INDEX` failed with "index … already exists". The clause was then reported Unknown. On the 300-file all-60 sample this caused all 22 baseline failures (10 after the rule rewrites, which only changed which names collide).

## 10. State-flow taint clause on narrow inputs

The state-flow clause of serverplan_state_tripwires (`state-flow-narrow`) now reads narrow inputs: one module gate, identifiers with their roles, string literals, and the node kinds the fixpoints read. It uses span containment for ownership. The ancestor ordering of sinks is reproduced by counting the `branches` rows that contain each event.

| | Before | After |
|---|---|---|
| Fixture parity (79 rows) | exact | exact |
| Clause cost over a no-clause variant, measured back to back | about 1.07–1.10 s | about 1.05 s |

There is no CPU gain, for two reasons:
- The old CST stages still feed `state-flow-project`, so the tree is still copied.
- On the sample only 24 files pass the gate. The clause's remaining cost is about 0.4 s of input extraction (an input not headed by one node kind walks the whole tree) and about 0.6 s of per-file SQL compilation and execution over small data.

The win arrives when `state-flow-project` also moves to narrow inputs and the tree copy disappears. An unconfirmed lead: the per-connection regex cache (64 entries, cleared when it fills) may churn on these many distinct patterns.
