# 0563: A relation no rule can fill makes no stratum, and an edit's delta reaches a rule by the kinds it holds

Status: designed (2026-11-21), spiked (2026-11-21, "Spike" below: each part validated on ep2, findings exact; the
design is amended there), not built. The user asked for it (2026-11-21): "okay, please survey and design this.
Does it also make sense to track changes per fact type? Presumably much work could be cut out if certain fact
deltas are empty without needing to propagate it." "This" is the 1,700 strata the merge of the serverplan-quality
hub (2ac89a3a70) added to every cold run of ep2, which makes no guests. Follows 0101 §1, 0117, 0120 and 0431.

## Intro

A run spends a fixed cost on each stratum it makes. It writes the stratum's spec, plans it and opens its inputs,
even when the stratum's rules can make no row. There are two sources of such strata:

- **Cold.** The findings' through-rule (0431 R9) and the hiding bound's guest rules are lowered for every program
  whose library *reads* a guest relation. They are lowered even when no guest can exist in the run. Their strata
  read `guest_place`, which is empty, and the demand and `pre_N` strata that feed them are made too.
- **Edit.** An edit reaches a stratum when it reads a relation whose delta has rows (0117). The tape's views,
  `node`, `kind` and `child`, change on every edit. So most maintained strata are reached, and almost none of them
  passes a row.

The design has two parts:

- **A.** At preparation, prove which relations no rule can fill, from inputs the run knows are empty. Then drop
  the rules that join them, and the rules that feed only those. They make no stratum.
- **B.** Track an edit's delta per kind of node: per fact type, as the user asks. A rule is reached only when the
  delta holds a kind that the rule's atom can match. The test runs before the stratum is rewritten or planned
  (0120 step 3).

**The answer to the question: yes.** Per-relation deltas already stop propagation at run time (0101 §1): a
stratum whose deltas are empty is skipped at its turn. But the skip comes after the stratum's fixed costs are paid.
The reach test also sees `node` as one relation, so it reaches every rule over nodes. Measured on ep2's `append`
edit, 534 maintained strata were reached, and 1 of them passed a row. A finer reach test is the cut. Making it
before the rewrite is what makes the cut pay.

## 1. Measured

All runs were on ep2, on the M3 (CPU). The profiles are in `tmp/ep2`.

### Cold

Compared: pk-53834 (before the merge) and pk-30951 (after the merge and da7ce72b95).

| Strata | Before | After |
|---|---|---|
| All | 3,631 | 5,322 |
| `pre_N` | 810 | 1,568 |
| `fz…`, `fzk…`, `xd…` `rr_diagnostic` | 3–6 each | 329 each |
| `demand_fzk…`, `demand_exists`, `xd_exists` | 0 | 163 each |
| File-local `violation`, `violation_at` | 168 | 330 |

- Join work is the same: 273.5 M rows.
- The extra strata cost:
  - 1.45 s CPU planning;
  - 3.5 s CPU in joins, at the same work (the fixed cost of each step and line);
  - 1.6 s CPU of "other: wait";
  - 1.2 GB of RSS (3.5 GB against 2.3 GB).
- ep2 makes no guests:
  - `guest_place` is empty;
  - the profile says "guests: no rule asks for a guest" (`corpus.rs`, `answering` is empty);
  - it also says "guest requests: made: a rule the outputs need reads guests".

### Edit (`append`, ed-60810)

- The edit took 7.34 s wall and 8.15 s CPU.
- Its cross part's strata:

| Strata by how they ended | Strata | Passed a row | Rows in |
|---|---|---|---|
| Held empty | 2,620 | 0 | 0 |
| Quiet (their kept rows) | 354 | 0 | 0 |
| Maintained | 534 | 1 | 2 |
| Run natively | 3,145 | 286 | 310,940 |

- Each stratum's turn cost about 0.34 ms of CPU:
  - the spec, 0.21 s over 3,136 strata;
  - the native step, 0.22 s;
  - opening inputs, 0.04 s.
  - In all, 1.05 s across the strata.
- Outside the strata the edit spent:
  - 0.47 s planning strata;
  - 0.44 s opening inputs;
  - 0.34 s on statistics.
  - These scale with the strata reached, not with the rows.
- 0117 step 3: 532 of 886 strata were reached by a changed read.

## 2. What makes a guest (survey)

A guest is a document parsed from a span of another document (0431). The engine makes guests in three ways:

1. **A request.** A rule derives one of `guests::REQUESTS` (`DEMAND`, `PARSE`, `SPLICE`), and the program reads an
   answer to it (0431 §8). `guests::answering(v)` lists these. A request's answer is admitted inside a chunk's
   fixpoint (`syntax::values::Layered`), so the guest relations are not final until the requests are answered.
2. **An embedding grammar.** A file's grammar embeds another grammar (0431 §4, `Dialect::embeds`,
   `Dialect::embed_places`), for example an HTML file's scripts.
3. **`.embedded on`.** A program reads other languages' files as guests (0512, `embedded_on`).

Today the engine decides "guests" in four places, and three of them use the wrong test:

| Where | Test | Right? |
|---|---|---|
| `corpus.rs` `guests_of` / `makes_guests` (~1841, ~1958) | a `REQUESTS` head, *or reads a guest relation*, or `.embedded on` | Reading is not making |
| `corpus.rs` `reads_guests` (~3765) | a needed rule reads `guests::READS` | Reading is not making |
| `lowering_tail.rs` `Findings::run` (`RulesSeen::reads_guests`) | the program or its library reads `guests::READS`, and `guest_place` and `tape_of` are declared | Reading is not making |
| `corpus.rs` `answering` (~4512) | a `REQUESTS` relation has a rule and its answer is read | Right, for requests |

The library reads `parsed`, so every program "reads guests". The findings pass then lowers the through-rule for
every program, and `hiding` makes every program's guest copies. Neither can find a guest in ep2.

## 3. Part A: a relation no rule can fill makes no stratum

### A1. One predicate: can this run hold a guest?

`guests::possible(library, programs, languages) -> bool` holds when any of these holds:

- a rule of the library or of a program derives a `REQUESTS` relation, and an answer to it is read (the test that
  `answering` makes today, made on the linked program before lowering);
- a language of the batch embeds a grammar (`embeds()` or `embed_places()` is not empty);
- a program is `.embedded on`.

The predicate depends on the library, the programs and the batch's languages. It does not depend on any file's
text.

- An edit keeps it, unless it adds a file of a new language.
- When it flips, the program is prepared again, as for a change to the library.
- The predicate is part of the prepared program's key (0073). So a kept plan or a prepared plan is never used
  across a flip.

When it is false, every relation in `guests::READS` is **empty by construction for the run**: `guest_place`,
`guest_of`, `sub_tape`, `parsed`, `host_node`, `parse_of`, `unparse_of`, `splice_of` and `tape_of`. The predicate
replaces the "reads" test in the three places in §2's table.

### A2. The closure: which relations can hold rows

Dropping only the through-rule would leave its demand rules, its `pre_N` and its `_exists` rules standing. The same
mistake would come back with the next feature that reads a relation that is sometimes empty. So the drop is one
pass over the linked program, run before strata are formed. Two fixpoints over the rule graph do it:

1. **Forward: what can hold rows.**
   - Seed it with every input, less those the run knows are empty (A1's set).
   - A rule can fire when every positive atom of its body can hold rows.
   - Its head can then hold rows.
   - The closure runs over rules, not rows. It costs microseconds.
2. **Backward: what is needed.**
   - Seed it with the outputs.
   - A relation is needed when a rule that can fire, with a needed head, reads it.
   - Demand relations of magic sets are covered by this: `demand_fzk…` is read only by `fzk…`'s rule. When that
     rule cannot fire, nothing needs `demand_fzk…`, and its rules go too. So do `pre_N` and `xd_exists`.

The program keeps only the rules that can fire and have a needed head. A relation no kept rule writes, and that is
no input, is declared and empty. Its readers see no rows, as they would at run time.

**Negation and aggregates.**

- A negated atom over a relation that cannot hold rows is true. The rule keeps firing, and the atom is dropped from
  it.
- An aggregate over such a relation:
  - `count` and `sum` give 0, so the rule stays;
  - `min` and `max` bind nothing, so the rule cannot fire.

**Why it is exact.** A relation that no rule can fill holds no row in any run that A1's predicate admits. A rule
that joins it makes no row. Removing both changes no output. It removes only strata, plans and statistics. This is
the empty-rule skip (0090 step 5, 0094 §4, `setfix::held`), proved once at preparation instead of found at each
stratum's turn. That skip refuses inputs ("An input may be bound when its first reader runs"), because an input's
rows come later. A1 proves the guest inputs empty before any row is bound. That is the only kind of input the
closure takes as empty.

### A3. Where guests can exist: inputs empty once admission has closed

When `possible` holds, a guest relation can still be empty for a chunk. Its rows are admitted only while requests
are answered (0431 §8). Once the last stratum that answers a request has run, the chunk's guest inputs are final:

- A guest input then counts as empty by its counted rows (`batch.rows.counted`). It never counts as empty by
  `len()`, which can make every lazy document (Q332).
- The strata after that point join the empty-rule skip as held relations do.

This is a run-time test, so it saves the run, not the planning. A1 and A2 are what remove ep2's 1,700 strata.

### A4. Targets

- ep2 cold: the strata count is 3,631 (±1%). The 163 programs' `fz`, `fzk`, `xd`, `demand_*`, `_exists` and extra
  `pre_N` strata are gone.
- The 6.5 s of CPU in §1 is recovered, and the RSS is back near 2.3 GB.
- Findings are equal to the run before. They are a set of finding keys.
- A test: a library that reads `parsed`, run with no embedding language and no request. Its prepared program has no
  rule over `guest_place`, and its findings equal those of a run of the same library and files without the closure.
  A second test runs the same library with a request answered. There the through-rule is kept, and a finding at a
  guest's node is placed at its string.

## 4. Part B: an edit's delta reaches a rule by the kinds it holds

### B1. The fact type is the node's kind

The relations that change on every edit are the tape's views: `node`, `kind`, `child`, `field`, `line` and the
rest. They are reads of the tape (0353). Rules read them by kind:

- a kind-set atom;
- `kind(n, "call")`;
- `node(n, K, …)` with `K` a constant.

ERROR is a member of every kind set that names it. So the fact type to track is the kind of the node a row is
about.

- **The rule's side.** Preparation gives each atom over a tape view a **kind demand**: a bitset over the grammar's
  kinds, of the kinds its node variable can take. It comes from the rule's kind literals and kind-set atoms, joined
  through `child` and `field` where the body names the child's kind. With no constraint, the demand is every kind.
  It is computed once and stored with the prepared stratum.
- **The delta's side.** When the changed documents' rows are bound as the delta (0117 step 2, `r__plus` and
  `r__minus`), the kinds the delta's rows hold are written once beside it, as a bitset per view. The bitset is a
  derived fact, written where it is computed: read from the changed range of the tape, never from a second
  structure.
- **Reach.** A rule's delta atom over a view is reached only when the two bitsets intersect.
  - A derived relation's delta is reached as today, by its rows (0101 §1).
  - So the cut propagates. A stratum none of whose rules is reached adds no delta, and its readers are not reached
    through it.

Per-constant partitions of derived relations (a finding's code, a field's name) take the same shape: a column a
reader binds to a constant, with a bitset or a small set of the values in the delta. They are left out of the first
step. The tape views are where the measured reach is (§1).

### B2. The test runs before the stratum costs anything

0120 step 3's driver walks the prepared strata and holds the set of non-empty deltas. B adds the kind bitsets to
that set:

- A stratum is rewritten, specced, planned and opened only when one of its rules is reached.
- A stratum that is not reached is its kept rows. This is today's "quiet", without the spec and the plan.

The driver is what makes B pay. Today the 534 maintained strata, the 354 quiet ones and the 2,620 held-empty ones
each pay the spec, the plan lookup and the open: about 0.34 ms each, and the planning, the opening and the
statistics around them (§1).

### B3. What has to hold first

B saves work only if an edit's delta of `node` holds the nodes that changed, not every node of the changed file.
If an `append` renumbers the file's nodes, the delta holds every kind of the file, and the bitsets intersect almost
always.

- So the first step of B measures the `append` edit's delta: its rows, and its kinds per view.
- If the delta is the whole file, the work is to make node identity stable across an edit (the delta's rows are
  the changed ones), before any bitset. That belongs to 0117 step 2's input deltas.

### B4. Targets

- On the `append` edit:
  - the strata reached fall from 886 toward those that pass a row (§1: 287);
  - each unreached stratum costs no spec, no plan and no open.
- The goal is under 1 s for the edit's cross part, against 2.35 s.
- Findings are equal to a cold run's after the edit. They are a set of finding keys.
- A test: an edit that changes only a node of a kind no rule of a stratum demands leaves that stratum unreached and
  its kept rows unchanged. The same edit to a demanded kind reaches it.

## 5. Order of work

1. **A1 and A2:**
   - the predicate and the closure;
   - the three call sites in §2's table switched to the predicate;
   - the tests in A4;
   - the cold run of ep2, measured against pk-53834.
2. **A3**, the guest inputs counted empty once admission closes. Measure it on a corpus with guests.
3. **B3's measurement** of the `append` delta. Then either the stable node identity, or straight to B1.
4. **B1 and B2** on 0120 step 3's driver. 0120 steps 3 and 4 are built first, or with it.

## Spike (2026-11-21): what holds, what changes

The user asked for it (2026-11-21): "spike on the entire shape of the solution … I want to validate that it will
work before we build the entire thing". All runs were on ep2, on the M3 (CPU). Each edit's outputs were compared
with the same edit's run without the spike, byte for byte (`outputs.rro`, and every relation's row count).

### A1 and A2: the predicate and the closure

In the tree, off unless `RERAIL_UNFILLABLE=guests` is set:

- `preparation::prune::Unfillable` is the forward pass. `preparation::unfillable` runs it, then `Prune`.
- `run_observed_within` runs both on the file-local and cross-file versions, where the run finds that the file-local
  part answers no request and is not `.embedded on`.

**The predicate moved.** On the combined program, `guests::answering` is not empty for ep2:

- the cross-file part derives `rr_embed_req` (0470's call-argument embedding: Python embeds HTML), 17,925 rows of
  it;
- but the file-local part, the only one that answers a request (0431 §8), derives none, and the profile says "no rule
  asks for a guest".

So the right test is §3 A1's, made on the file-local part after the split: `answering(local_version)` is empty and
the run is not `.embedded on`. The run already makes that test, as `parsing`. A1's "a language of the batch embeds"
is not needed: an embedding is made only by a request or under `.embedded on`. The requests ep2 makes are never
answered, so they are work for nothing too. A request no part answers can go the same way, as an empty input of its
readers.

**The empty set is narrower than `guests::READS`.**

- `guests::READS` has `tape_of`, `doc_grammar` and `rr_program`. These are a file's too: `sources.rerail` reads
  `tape_of` for every loaded document.
- With them taken as empty, the `sources` phase made no `document`.
- The guest-only set is `guest_place`, `guest_of`, `host_node`, `sub_tape`, `parsed`, `parse_of`, `unparse_of` and
  `splice_of`. The build names it in `guests` beside `READS`.

**A relation is empty only when it traces back to an empty input.**

- The first closure took a relation that only rules make as empty when none of its rules could fire.
- A phase's engine-bound relations (`load`'s) broke that.
- The closure now seeds from the empty inputs alone: a relation every rule of which joins an empty relation is
  empty. It is conservative for recursion through an empty relation, and exact for ep2.

**Measured, cold.**

- File-local part: 1,516 rules can never fire and `Prune` takes 414 more (6,637 → 4,707 rules), 1,328 relations
  empty.
- Cross-file part: 3 rules and 103 more (3,377 → 3,271).
- Strata: 8,067 → 5,608 (5,050 before the merge).
- The `fz`, `fzk` and `xd` `rr_diagnostic` strata are back to their counts before the merge (329 → 3 and 6), and so
  are `pre_N` (1,568 → 844) and `violation_at` (329 → 166).
- Still added by the merge: `demand_exists` and `xd_exists` (163 each) and `flN_violation` (330 against 168). They
  are not guest rules: `exists_N(d) :- pN_X(v), node(d, v, …), !placed_at(v)`, whose `xd_exists` is found empty only
  by its data.

| ep2 cold, plain, interleaved after a warm-up | Instructions | CPU | Wall | Peak memory |
|---|---|---|---|---|
| Off (two runs) | 733.0 / 732.7 G | 57.9 / 55.3 s | 20.1 / 18.8 s | 1,714 / 1,730 MB |
| On (two runs) | 717.6 / 717.3 G | 55.4 / 53.8 s | 18.8 / 18.1 s | 1,660 / 1,661 MB |

**A4's targets were wrong.** The saving is 2.1% of instructions and about 2.5 s of CPU, not 6.5 s and 1.2 GB:

- the profiled runs' per-stratum costs (§1) were mostly the profiler's own;
- the first run of a session costs about 130 G more than the next, so order an A/B after a warm-up.
- The gain is real and exact, and the rules removed (1,930 of 10,000) cost preparation, planning and memory.

### A3: not validated

In ep2 no part answers a request, so the static prune covers it, and A3 has nothing to cut. Validating A3 needs a
corpus whose file-local part answers requests. It stays as designed, and is built after A1–A2 and B.

### B3: what an edit's delta holds

Traced at the driver's input deltas (`driven_part`'s `answer`):

| Edit | `node` delta | Kinds | Largest node-keyed deltas |
|---|---|---|---|
| `append` (a function at a file's end) | +14 −1 | 11 | 13 rows of 10 kinds each (`ast_depth`, `node_order`, `visit_index`, `unparen`, …) |
| An import at a file's top | +4,341 −4,336 | 71 | `line` +4,678, `node_order` and `visit_index` +4,340 |

- A row names its node by preorder (`input_delta`), so an insert renumbers every node after it, and the delta is the
  rest of the file.
- The kind cut still pays there (below), but stable node identity would shrink the delta itself. That is its own
  item: 0117 step 2's input deltas, after B.

### B1–B2: the reach by kind, and by key

The spike tested, at the driver's reach (`plan.rs`, where `moved` is formed), whether every rule of a reached stratum
sees no row of any changed read's delta. If so, the stratum takes the quiet path. Two tests, per atom over a changed
read:

1. **By kind.**
   - The atom's node variable is held to kinds by `node`'s or `kind`'s kind constant, or by a kind set's facts.
   - The constants are branded atoms (`#k1230`), whose values are the tape's kind codes, the same values as `node`'s
     delta holds.
   - `kind` is a projection of `node`, so its delta is `node`'s, projected.
2. **By key.**
   - Another positive atom of the rule over a relation that did not change shares the variable.
   - No value of the delta's in that column is one of that relation's rows there.
   - It is a semijoin of the delta's few keys against the relation's index.

**Kind alone is not enough.** Of the 536 strata `append` reached, 325 were reached through `node` by the merge's
`exists_N(d) :- pN_X(v), node(d, v, _, _, _), !placed_at(v)`. These hold no kind. Their selectivity is `pN_X`, a
finding's nodes. The key test cuts them. So "per fact type" is per kind for the tape's views, and per key for
everything that joins them.

**Measured** (profiled edits; the outputs identical in both):

| ep2 edit | Strata cut, of those reached | Maintained strata | Joins of the cross part | The spike's probe | Instructions |
|---|---|---|---|---|---|
| `append` | 449 of 552 | 531 → 82 | 1.30 s → under 0.2 s | about +0.9 s | 83.5 → 77.2 G |
| Import at the top | 391 of 558 | 377 → 156 | 1.26 s → 0.41 s | about +1.0 s | 87.5 → 86.8 G |

- The cut saves about 0.9 s of joins on each edit, of a cross part of 1.8–2.0 s.
- The spike's probe costs about as much again, under "other: cross". It reads each probed relation's column whole
  into a set, and admits a stored relation (`Maintain::need`) to do so.
- That is also a copy of a block (0353), so the spike's B code is not in the tree. It is kept as a patch,
  `tmp/spikes/0563-spike-b-plan.patch`, and with the B3 trace in `tmp/spikes/0563-spike-all.patch`.

**What the build does instead.**

- **The delta's keys and kinds are derived facts**, written once where the delta is made (`driven_part`). They are
  never re-read from the delta's rows per atom.
- **The key test probes the relation's own index in its block** (`Source::lookup_read` or `sorted_rows`). A stored,
  unread relation is probed through a key summary kept with its frames (its keys' filter, written when the
  relation is kept), so the test admits no rows.
- **The test runs before a stratum's spec and plan**, as the spike's did at the driver.

The target stays: the cross part's joins less 0.9 s on both edits, at a probe cost of a few ms. The test cuts a
stratum only where its proof is exact (the outputs compared). For the other 103 strata `append` reached, either a
rule's key is a column of a relation changed in the same edit, or the read has no given delta (a relation derived
in the run, whose delta is known only when its stratum has run).

### The order of work, amended

1. **A1 and A2** as spiked:
   - the predicate on the file-local part (`!parsing`, not `.embedded on`);
   - the guest-only set named in `guests`;
   - the closure seeded from the empty inputs;
   - a request no part answers taken as empty;
   - the tests of A4.
2. **B**:
   - the delta's kinds and keys made with the delta;
   - the kind and key tests at the driver, probing indexes and key summaries;
   - the outputs compared on the `append` and insert edits, and B4's test.
3. **Stable node identity** across an edit, so an insert's delta is the nodes changed.
4. **A3**, on a corpus that answers requests.

## Prior art

- **Soufflé** removes relations that no rule can fill, and rules that join them, before evaluation. Its AST
  transformers `RemoveEmptyRelations` and `RemoveRedundantRelations` are a forward and a backward pass like A2. Its
  magic-set transformer runs before them, so the demand relations of an empty predicate go with it.
- **Magic sets** (Bancilhon, Maier, Sagiv and Ullman, 1986). A demand relation exists only to restrict its
  predicate. When the predicate can hold no row, its demand is dead code.
- **Irrelevant updates** (Blakeley, Larson and Tompa, "Efficiently updating materialized views", SIGMOD 1986), and
  queries independent of updates (Levy and Sagiv, VLDB 1993). An update that cannot satisfy a view's selection is
  detected from the update and the view's predicate alone, without evaluating the view. B1's kind demand against
  the delta's kinds is that test, with the selection taken as the kinds a rule's atom can match.
- **Differential dataflow and DBSP** (Budiu et al., VLDB 2023). An operator whose input batch is empty does no work,
  and filters are pushed into the delta streams. This is 0101 §1's cut. B moves the filter in front of the
  operator's fixed cost, as these systems do by making each operator's schedule free when its input is empty.
