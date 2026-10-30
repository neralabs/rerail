# The first end-to-end slice

Status: plan, 2026-10-29.

- Architecture: `03-architecture.md`.
- Workplan: `05-implementation-plan.md`. The copying stage is complete; this is the first architecture step.
- Open items: `open-items.md`.

## Why this slice

Every component of 03 is in the tree and tested in isolation. Production still runs W's engine: per-file workspace evaluation with W's matcher. Outside test code, only `prefilter::probes::TextProbe` and `residency::admission::census` are reached. 03's targets are measured on the path below, and it exists nowhere end to end:

> cold audit 1–2 s, warm run after a small edit in milliseconds, 500 MB RAM.

The slice builds that path once, for one rule family, in the target shape. It is a small batch of the real plan, not a different execution shape. It runs beside W's engine and is judged by finding keys (`findings-contract.md`, the loose contract).

**Scope:** Python `forbid` clauses whose `ast` rule is file-local. That means no `calls`, `resolves`, `imports` or `signature`, and no file-level gates (`file.package`, `file.path`).

**Size:** the Python packs have 286 `ast` clauses (234 policy files). Only 54 of them are plain `kind`/`pattern`/`regex`. The others use:

| Operator | Clauses |
|---|---|
| `kind` | 195 |
| `matches`/utils | 151 |
| `stopBy` | 137 |
| `pattern` | 137 |
| `any` | 134 |
| `has` | 121 |
| `not` | 109 |
| `all` | 109 |
| `regex` | 84 |
| `inside` | 81 |
| `follows` | 55 |
| `count` | 28 |
| `precedes` | 27 |

So the slice must lower composite matchers, not just patterns.

## The shape

```
  Git root tree (or a directory snapshot)          edit: one blob rebound
        │                                                  │
        ▼                                                  ▼
  facts::observe::push_root ── rebinds names ─► facts::propagate (waves over derivations)
        │                                                  │ recompute(frontier)
        ▼                                                  ▼
  per changed blob, one derivation each:  source → tape (v8) → core facts → prefilter rows
        │                                  (content_store, prefilter::publish)
        ▼
  wave 1 (one SQL-driven step, waves::schedule::run):
     prefilter::select(store, python, plans, tapes)        candidate (tape, preorder) per clause
     demand(batch) = PackedDemand over the lowered programs   CPU interpreter; Metal interpreter
     match rows → match facts (demand::driver), one family per (rule, clause)
        │
        ▼
  findings layer: one SQL statement over match facts → FindingKey set + Coverage
  (findings::contract)                                     cutoff: an unchanged family stops here
```

Five decisions of 03 are exercised on this path:

1. **Rules go through the logic AST.** `logic::front::yaml` already reads every pack document into the logic program, with a `forbid` as `violation(…) :- ast($D, $N, {…})` (Phase 6 row 3). The slice takes each `ast(...)` atom from there, not from W's `compile/*` output. Matcher rules therefore lower from the same program that generic rules will (§0, §12). The `violation` rule becomes the findings SQL (§8).
2. **Matchers lower to transducers, combined per batch.** Each atom becomes one packed `Program`. All programs of the wave go into one `PackedDemand`, executed once per (group, batch), never per file (§3, `shape-over-completion`).
3. **The prefilter chooses the roots.** Anchors come from `prefilter::select` with the statistics already in the store (§6). The matcher does not scan every node.
4. **The push path drives everything** (§2, §14). Cold and warm are the same code: cold rebinds every name, warm rebinds one. Early cutoff on `output_hash` is what makes warm runs cheap.
5. **SQL controls the wave** (§4, §7). The step is a `waves::schedule::Step` whose SQL reads `demand(batch)`. A file-local program needs one wave; the loop is the general one, with `max_waves` = 1 asserted for this rule family.

## Steps

Each step is one or a few commits. Each is checked with `cargo check` and the touched module's tests (seconds), and pushed. No PR.

### S1. Lower W's matcher to packed programs — done (2026-10-29, decision 0011)

**Files:**
- `matcher/lower.rs`: W's `AstMatcher` becomes a `MatchTree`, or is refused by operator.
- `transducer/packed/compile/matcher.rs`: `LoweredMatcher`, the label passes and the root program.
- `TapeDoc::with_labels`: derived classes as bitsets.
- `compile/matcher_tests.rs`: the gate.

**Shape.** A clause is a sequence of node-label passes, then one root program.
- Labels are derived classes that later programs test with `SUPERTYPE`, so no new operation is
  needed.
- **Negation** (`not`, a stop rule's bound) is a complement label between passes, not an
  operation. This replaces S2's NOT_EXISTS barrier, as the Bend investigation proposed:
  "sequence full-width phases, do not fork them against each other".
- **Programs are positive** and walk one relation deep. Backward walks (`inside` to the end or a
  stop, `follows`) are forward passes from each match, because the verifier admits only loops that
  move forward in preorder.
- **Recursive utilities** are groups iterated to their fixpoint. Through `not`, only recursion
  that walks forward (`has`, `precedes`) is admitted, which is structural descent as the
  termination proof.
- **Each pass starts from its domain:** the rule's kinds, or a label.

**Gate.** `lowered_python_clauses_match_where_ws_matcher_does`:
- For every Python pack clause that lowers, and every fixture, the set of root nodes equals
  `GrammarMatcher::match_root` at each node of the same tape.
- It runs every eighth fixture by default; `RERAIL_S1_ALL=1` runs all of them.
- `the_gate_catches_a_widened_walk` is the mutation check: widening a `neighbor` walk to `end` must
  fail the gate.

**Result:** 77 of 286 clauses lower, and 77 of 77 are exact on all 475 fixtures. Refused:

| Refusal | Clauses | Where it goes |
|---|---|---|
| `resolves` | 94 | O24 (semantic labels) |
| `count` | 25 | S2 (10 lower; 15 refused for semantic leaves inside) |
| shared binding | 18 | O24 (joins on captures) |
| `parse` | 13 | O24 |
| `arguments` | 13 | O24 |
| constraints below top level | 12 | O24 |
| `resolves.binding` | 11 | O24 |
| `formatArgs` | 6 | O24 |
| `value` | 6 | O24 |
| `binds` | 5 | O24 |
| parameterized utility | 3 | later |
| `layout` | 2 | O24 |
| `compare` | 1 | later |

**Cost.** Over all 475 fixtures, the lowered passes take 83.6 s against W's matcher's 5.5 s, about
15× (wall time inside the test, in a debug build of the crate, from every node; the run took 66 s
of user CPU at load average 27). That is generic interpretation (spike 2.23) without
anchors. S4's prefilter gives the root program its candidates, and the JIT (Phase 4d) replaces the
interpreter.

### S2. COUNT, and the labels on Metal — done (2026-10-30)

- **`count`** (without `as`) is a counting label (`Label::Count`, `Builder::count`). The counting
  program has one path per node the walk reaches that the rule's label has. A forward walk runs
  from every node and is tallied by its root. A backward walk (`inside` past the parent,
  `follows`) runs forward from each match and is tallied by the node it reaches. The tally,
  compared with `min..=max`, is the label. The rule is always a label, so an `any` cannot fork one
  node into two paths.
- **Labels on Metal.** `TapeDoc::transducer_supertypes` lists each label's nodes as members of its
  derived class, so Metal's batch uploads them in the class table it already has (no new buffer,
  no new operation). `LoweredMatcher::roots_batch(backend, tapes)` runs each pass once over the
  whole batch: one Metal interpreter dispatch per program pass, with the interpreter compiled once
  per process (`compile_interpreter_cached`).
- **Gates:**
  - `lowered_python_clauses_match_where_ws_matcher_does` (CPU against W): 87 of 286 clauses
    lower, all exact. That is 10 more; the other 15 `count` clauses are refused for semantic
    leaves inside them.
  - `lowered_clauses_find_the_same_roots_on_metal` (`metal-tests`): 87 of 87 clauses give the
    same root sets on CPU and Metal (4 fixtures, 85 matched nodes).
  - Both take `RERAIL_S1_CLAUSE` (a clause-name filter) and `RERAIL_S1_FILES` (the first n
    fixtures) for targeted checks. The default sample is every eighth fixture.
- **Left in O23:**
  - Pattern leaves and the host passes (complement, constraints, count tallies) run on the
    host, as `demand`'s pattern producers already do.
  - Each program pass is its own dispatch with a host round trip. Metal took 77 s against the
    CPU's 7.7 s for 87 clauses on 4 fixtures, debug build, load 12.
  - One command buffer for all of a batch's passes, with the host passes as kernels, is the
    remaining work. It is measured at S8.

### S3. Ingest through the push path (closes O2's ingest side)

- The `recompute` callback of `facts::observe::push_root`, for blob names of Python files, does four things:
  1. reads the source (CAS);
  2. encodes the tape (v8, `tape_objects`);
  3. builds core facts (`facts::core`);
  4. publishes prefilter rows (`prefilter::publish`).
- Each output is a derivation with its `output_hash`, so an unchanged tape cuts off.
- Observers: a directory snapshot builds a tree (W `git_workspace` already writes `git_trees`); a Git root comes from `HEAD`.
- Check: Phase 1's frontier test, extended. One blob edit re-encodes exactly one tape, and its unchanged kinds cut off.

### S4. The wave: prefilter → `demand(batch)` → match facts

- One `waves::schedule::Program`, `file_local = true`, with one step.
- **Step:** SQL reads the candidate sets from `prefilter::select` for every lowered clause's `AnchorPlan`, and calls `demand(batch)` over the packed programs (CPU interpreter).
- **Output:** match rows are written as a match-fact family per (rule, clause) in the overlay (`demand::driver`), with row estimates in `fact_stats`.
- **Chunks:** 125 MB chunks, prepared ahead (decision 0010).
- This exercises the scheduler and the vtab on real fact families. That is part of O14, with a hand-written step rather than one emitted by the planner.
- Check: the batch's candidates must be a superset of S1's oracle roots (the prefilter never under-produces, §16.3), and the match facts must equal S1's matches.

### S5. Findings layer (first version of §8)

- One SQL statement over the match-fact families. It joins the clause's violation shape (anchor, code, message key) and emits `FindingKey`s, with Coverage `Complete`, or `Unknown(resource)` on a budget or limit.
- `findings::contract::Findings::publish` writes the set as a family, so an unchanged finding set cuts off.
- Check: the finding keys equal W's `violations` for the same files and clauses, as sets.

### S6. Entry points and the warm path

`slice::audit(store, scope, root, rules)` and `slice::apply(store, scope, edits)`:
- both call `push_root` and return findings;
- tests and a bench binary only, behind no feature, since nothing in production calls them yet;
- a napi method comes after the gate (S7) passes.

Check: an edit to one file re-runs exactly that file's derivations and its clause matches. Its findings change only for that file. A whitespace-only edit cuts off at the tape (same structure) or at the matches.

### S7. Differential gate against W's engine

- **Sample:** 100–300 real Python files, not fixtures only. Spike 2.18: measure at deployment size, and fixtures hide scaling (DH L3630-3635). The `packs/python` fixtures plus one real repository slice.
- **Rules:** every Python clause S1/S2 lowers.
- **Report:**
  - finding-key set equality, per clause;
  - coverage of the 286 clauses;
  - the answer count next to every time (spike 2.18, DH L84-87).
- Mutation check: shift a span by one byte and break one rule, and the gate must fail (spike 2.17).

### S8. First measurements against the targets

On the S7 sample, with CPU time (user + sys) and the machine load noted:
- cold audit;
- a warm one-file edit;
- a warm whitespace-only edit;
- peak RSS.

Interleave the arms and discard the first pass (spike 2.18, DH L5656-5680). This gives the first real numbers for 03's targets, and the counters §10 answer 4 asks for: re-runs against actual changes.

## What the slice decides, and what it leaves

**Decides:**
- **O2:** the push path, prefilter and scheduler are called on real data.
- **The O19/O20 anchor question, for production:** the slice uses W's `matcher::anchors::AnchorPlan` with `prefilter::select`, not P's or D's anchor engines. Those stay test-only until O20 closes.
- **O14, partly:** a wave step over `demand(batch)`, still hand-written.
- **The Phase 4 write-new items S1/S2 need:** lowering composite matchers, which no stream has (G's `AstMatcher::Program` wraps one compiled pattern); and the barriers.

**Leaves, deliberately:**

| Left out | Where it belongs |
|---|---|
| Cross-file clauses (`calls`, `resolves`, imports) | multi-wave |
| The planner emitting steps | O14 |
| JIT and the interpreted→JIT swap | Phase 4d |
| The combined hash-consed graph across rules | Phase 4c; S4 batches programs but does not merge them |
| The Metal grid backend and the pinned slab cache | Phase 9 |
| `R[tid]` regex | Phase 4g |
| Typing | Phase 10 |
| Rules that carry SQL | Phase 11 |

None of these changes the shape. Each one replaces a part of this path in place.

## Risks

- **Matcher semantics are subtle.** `stopBy` defaults, `inside` versus `has` with fields, `nthChild`, ERROR and MISSING nodes, and three-valued utilities are all involved. S1's per-root differential is the guard: a clause lowers only when it matches W exactly, and it is refused otherwise. No clause is reported as covered without parity (spike 2.17: never catch residuals).
- **Barriers on Metal (S2).** If the Metal interpreter cannot express a barrier boundedly, it is an F1 violation: record it, and do not fall back to CPU only.
- **Prefilter under-production.** S4's superset check on every clause; the vocabulary path has a scope-limited fallback for kinds it does not cover (§16.3, `b839fa05`).
- **Cutoff granularity.** Per-file derivations may re-run more than needed. S8's re-run counter measures this (§10 answer 4).
