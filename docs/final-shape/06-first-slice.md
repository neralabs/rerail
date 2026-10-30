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
- **Labels on Metal, every pass on the GPU** (`metal_program_backend::label_passes`,
  `metal_label_passes.metal`, `metal_label_patterns.rs`, `LoweredMatcher::roots_metal`):
  - **Storage.** The labels live in a prefix of the batch's supertype buffer: element 0 is
    {labels, words per label}, then one bitset per label over the batch's global nodes.
    `has_supertype` reads a class at or above `0xf000` from the prefix, so `SUPERTYPE` tests a
    label on Metal as on the CPU. No new operation.
  - **Pattern leaves.** A clause's ast-grep patterns become one generated kernel. Each
    `PackedPattern` turns into functions that mirror `PackedPattern::one`/`children` line for
    line; a pattern is a tree, so no function recurses. A repeated metavariable's structural
    equality walks an explicit stack, and one too deep for it fails the clause. Constraints are
    checked in the kernel against the label bits.
  - **Levels.** Passes run a dependency level at a time. A level's labels read only earlier
    levels' labels. Each level is:
    1. one interpreter `run_batch` dispatch with the level's programs, on one shared workspace
       whose scheduler image a blit copies in;
    2. `label_sink`: results become scratch bits, or count tallies routed by task;
    3. the pattern kernel;
    4. `label_settle`: each target takes its scratch bits, or a complement or count bound
       computed per word, and a change sets a flag.
  - **Recursive groups.** A group's rounds are encoded in the same command buffer. A node
    joins a recursive label in the round after the node its walk reaches, and both have the
    label's kinds. So the count is the longest chain of nodes of those kinds, nested along a
    path (or side by side where the group walks siblings), plus two, within the launch budget.
    Where the label's kinds are unknown, every node counts. A one-thread `label_round` kernel marks every round after one that changed
    nothing as skipped, and every label kernel and `run_batch` return at once on that mark. The
    decision is taken by thread 0 of each round's last `label_settle`, for the round before
    (one changed flag per round parity), so it costs no launch. Deciding a round late only
    spends a round that changes nothing. One `label_round` per group records whether the group
    settled; a group still changing at its last round reruns the clause with more rounds. That
    rerun is counted, and none happened on the fixtures.
  - **One command buffer per clause.** Every level's batch is built once and the workspace is
    sized before encoding, so the host never waits to grow it. The host's only work before the
    command buffer is building the batches and the label-domain root lists; after it, it reads
    the label words.
- **Gates:**
  - `lowered_python_clauses_match_where_ws_matcher_does` (CPU against W): 87 of 286 clauses
    lower, all exact. That is 10 more; the other 15 `count` clauses are refused for semantic
    leaves inside them.
  - `lowered_clauses_find_the_same_roots_on_metal` (`metal-tests`): 87 of 87 clauses give the
    CPU's root sets on Metal on the whole default sample (60 fixtures, 1,177 matched nodes). It
    ran as four batches of 15 (`RERAIL_S1_FROM`) to keep each run under two minutes, and the CPU
    gate against W is 87 of 87 exact on the same batches.
  - Each batch is one command buffer per clause, with 0 host waits for growth, reruns or
    uploads. Launches average 69, 45, 51 and 36 per clause across the four batches, and are at
    most 559 for one clause, under the evaluation budget of 600.
  - On 8 fixtures (164 matched nodes), likewise one command buffer per clause and no host
    waits.
  - Metal took 15–23 s and the CPU 18–21 s over repeated 8-fixture runs (test wall time, debug
    build, load 13–18).
  - Encoding 4 rounds first brings launches down to about 62 per clause, but 3 clauses then
    need a rerun (90 command buffers). One command buffer per clause was kept.
  - `metal_catches_a_widened_walk`: on 16 fixtures, widened clauses differ from W's matcher on
    Metal and still agree with the CPU.
  - All gates take `RERAIL_S1_CLAUSE` (a clause-name filter), and `RERAIL_S1_FILES` and
    `RERAIL_S1_FROM` (n fixtures from the m-th). The default sample is every eighth fixture.
- **Left for later (O23):**
  - A clause with a recursive group whose labels can have any kind still takes up to 559
    launches. Its rounds are bounded by the tree's depth or sibling runs, and a skipped round
    still counts. The launch budget's target of under 100 per evaluation
    (`metal_launch_budget.rs`) is S4's check, where a wave's clauses share one evaluation and
    its levels and rounds. The user confirmed on 2026-10-30 that this target belongs to S4,
    not S2.
  - A label-domain program starts at every node, because its label is not known when the
    command buffer is encoded. Its first `SUPERTYPE` drops the others.

### S3. Ingest through the push path (closes O2's ingest side) — done (2026-10-30)

- `facts::families::Families::push(graph, scope, root, max_waves, others)` is the push: it
  observes the root (`observe_root`), propagates every rebound name, then creates the families of
  each new path, 256 paths at a time, wave by wave. Cold and warm are the same call: cold, every
  path is new. `others` re-runs the waves' derivations of other producers (the families' readers).
- Four derivations per file with a tape dialect, each with its `output_hash`:
  1. `#tape:` reads `#path:`. It reads the source (CAS, by the blob's Git id), encodes the tape
     (v8) and stores it in `tape_objects`. Its digest is the tape's canonical hash. Content
     already stored (another path, another scope) is decoded, not encoded again.
  2. `#core:` reads `#tape:` and builds core facts from that tape (`core::from_tape`); it no
     longer parses.
  3. `#prefilter:` reads `#tape:` and publishes the tape's rows (`prefilter::image`, 64 tapes per
     transaction). Its digest is the tape's and the rows' image version.
  4. `#kinds:` reads `#core:`, as before.
- A tape is kept in memory from the tape wave until its two readers take it, and facts until
  kinds takes them. A reader that finds neither decodes the stored tape, so a wave never encodes
  twice. A chunk's tapes are resident, not a scope's.
- Observers (`facts::observe::roots`):
  - `directory_root` hashes a directory into Git trees (SHA-1, Git's order and encoding), stores
    them and the sources of files with a language, each under its blob id.
  - `git_root` stores the `HEAD` tree from W's `git_workspace::discover`, and the committed
    sources from the repository's objects.
  - A clean checkout's snapshot equals its `HEAD` tree.
- Derived documents (embedded languages) are not stored by `#tape:`; Python has none.
- Check (`facts::families` tests, 4 tests, 0.1 s):
  - `a_directory_edit_reencodes_one_tape_and_its_unchanged_kinds_cut_off`: 3 Python files and one
    other.
    - Cold: 12 derivations created, 3 encodes, 3 prefilter sets.
    - A rename in one file: 1 encode, 0 decodes, 3 waves, 4 re-runs (that file's tape, core,
      prefilter and kinds), 3 changed. Its kinds digest is unchanged, so it cuts off.
    - An unchanged snapshot rebinds nothing. A copy at a new path decodes the shared tape (0
      encodes). A removed file unbinds its four names.
  - `a_clean_checkouts_snapshot_is_its_head_tree`: a committed repository (named `git add`)
    gives the same root by either observer, and its push encodes each file once.
  - Phase 2's tests now include the tape wave: a rename is 3 waves, 4 re-runs and 3 changes; the
    later reader runs on a kind change.
- Left for later: nothing outside tests calls `push` yet. W's workspace scan keeps its own path
  until the entry points (S6). Worktree edits reach the slice through a directory snapshot.

### S4. The wave: prefilter → `demand(batch)` → match facts — done (2026-10-30)

- **Program** (`waves::matches`): one `waves::schedule::Program`, `file_local = true`, with one
  step:

  ```sql
  INSERT OR IGNORE INTO new_match SELECT t.producer_id, w.d, t.node FROM rerail_terminal_match t
  JOIN temp.wave_document w ON w.position = t.file_index WHERE t.batch = :batch
  ```

  `max_waves` = 1 per chunk is asserted.
- **Per chunk, `Wave::demand`:**
  1. Reads the documents' stored tapes (S3's `#tape:`).
  2. Selects every clause's candidates in one `prefilter::select` statement over the tapes' rows,
     and keeps those of the root's kinds.
  3. Evaluates every clause's labels over the chunk together (`LoweredWave`). Each clause's labels
     are shifted past the clauses before it, so one label set per tape holds them all.
  4. Registers the 87 root programs, anchored at those candidates, as one `PackedDemand` on the
     CPU JIT, over the chunk's files in one execution. The packed image now lists a tape's labels
     as classes (`TapeDoc::transducer_classes`).
- **Scheduler changes:**
  - `temp.wave_document(position, d)` maps a batch's `file_index` to the chunk's document keys.
  - A batch that does not complete fails its chunk, instead of leaving its relations silently
    short.
  - `partitions` records `match` as one family per clause, `{scope}#wave:matches:match:{clause}`,
    beside the relation's own. The cost of each family is its row count.
- **Row estimates:** `Wave::statistics` writes each clause family's rows and documents to
  `fact_stats`. It also writes the most for `rerail_terminal_match`, which the demand relation's
  estimate reads.
- **Chunks:** the scheduler's 125 MB chunks, with the next chunk prepared while one runs (decision
  0010).
- **Checks** (`waves::matches` tests). The fixtures are ingested through the push path (S3).
  - `the_wave_finds_ws_matches_from_the_prefilters_candidates`:
    - For every clause and fixture, the candidates contain W's roots, and the match facts equal
      them.
    - On 15 fixtures: 87 clauses, 833 passes, 324 matched nodes, the same as S1 on those
      fixtures.
    - The candidates are 40,343 of 1,527,981 clause-nodes (2.6%). None under-produce and none
      differ.
    - It took 100 s on the CPU JIT, 30 s on the reference interpreter (debug build, load about 17).
      The gate's default is 8 fixtures; `RERAIL_S1_FILES` widens it.
  - `the_wave_runs_chunk_by_chunk`: the same check in chunks of one document each.
- **Metal check, met (O25 closed).** On Metal the wave is one evaluation
  (`LoweredWave::roots_metal`), checked by `the_wave_is_one_evaluation_on_metal`:
  - The result: 1 command buffer, 77 launches, no reruns. All 87 clauses' roots equal W's on all
    15 fixtures (324 matched nodes).
  - **Labels by global dependency level.** Every clause's labels are scheduled by global
    dependency level, merged across clauses. The non-recursive passes take 45 launches, about 15
    levels at 3 kernels each.
  - **Recursive groups sweep** (`label_sweep`) instead of iterating rounds. Each GPU thread takes
    a file and runs the group's tasks in node order. A result marks its label at once, so later
    tasks in the same sweep read it.
    - A node's labels run past the labels they read at that node (`local_levels`).
    - A group of positive programs alternates forward and reverse preorder.
    - A group with complements sweeps in reverse preorder only, computing each node from final
      verdicts (decision 0011).
    - The groups of a wave level are merged. Each costs 4 launches, one per sweep, and a sweep
      after one that changed nothing does nothing.
  - **Measured.** Before, the rounds moved the recursion one tree edge each, encoded to bounds
    of 25–154 (f704 needed 18). Now f704 settles in one sweep, and the S2 parity gate (87 clauses
    one at a time) takes 1,196 launches instead of 4,731, at most 47 for a clause.
- **Left for later, resolved (2026-10-30):**
  - **CPU cost.** The cost was not compiling (the JIT cache is per program) but the JIT's linear
    scan of supertypes. `SUPERTYPE` now binary-searches the memberships, and a derived class is
    one bit test: `PackedBatch` carries the labels as bitsets (`derived`), in place of
    `TapeDoc::transducer_classes`. The CPU's labels run by the levels the Metal plan uses
    (`levels`), recursive groups as sweeps in node order, on the interpreter. The root programs'
    execution went from 12.5 s to 78 ms on 4 fixtures; on 15 fixtures the whole wave takes 19.1 s
    (CPU 21.3 s, load about 17), from 100 s.
  - **Background compilation** (decision 0012): `CpuBackground` and `MetalBackground` run a batch
    on its compiled programs once they are ready, and on the interpreter until then, with the
    compilation requested on a thread. Metal builds its interpreter (and the sweep kernel) as part
    of the compilation. Checked by `the_wave_runs_while_its_programs_compile` and the
    `compilation` tests.
  - **Root programs on Metal.** `Wave::demand` on a Metal backend evaluates labels and roots with
    `LoweredWave::labelled_metal`; `the_wave_finds_ws_matches_on_metal` (8 fixtures, and chunks of
    one in the background mode) has 0 differ. The roots take 178 ms; the label passes before them
    are slow (O26).
  - **Planner.** `logic::plan::emit` gives a lowered program as a `schedule::Program`, and T's
    workload, workload_demand and calls give `run_atoms`' outputs through `schedule::run` (O14).
    The wave's own step stays one statement: it reads the demand relation, and is not a rule.

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
