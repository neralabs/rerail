# 0563: A relation no rule can fill makes no stratum, and an edit's delta reaches a rule by the kinds it holds

Status: designed (2026-11-21), spiked (2026-11-21, "Spike" below: each part validated on ep2, findings exact; the
design is amended there), A1–A2 and B built (2026-11-21, "Built" below). A1–A2 hold where no document embeds a
program: its language embeds none, or the store records its text as loaded with none, or (a text not loaded yet)
its language embeds only by call argument and the text names no place's callee. So they hold on ep2. The user
asked for it (2026-11-21): "okay, please survey and design this.
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
the run is not `.embedded on`. The run already makes that test, as `parsing`. The requests ep2 makes are never
answered, so they are work for nothing too. A request no part answers can go the same way, as an empty input of its
readers.

**Corrected in the build: A1's "a language of the batch embeds" is needed.** The spike claimed that an embedding
is made only by a request or under `.embedded on`. That is false: a language's own embeds are made when its
document is loaded, with no request (0431 §4). An HTML file's scripts and styles are guests of their file, and so
is a Python call's HTML argument (`Markup("…")`, `format_html(…)`, 0470's call-argument embedder). The spike's
predicate held on ep2 only because none of its files has such a call. Its outputs were the same, but the proof was
not.

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

- ~~A row names its node by preorder, so an insert renumbers every node after it.~~ Corrected in the build: node
  ids are by definition already (`granules`, 0083 §3 and 0094 §2, the default), and the insert moves no other
  granule's ids. What changes past an insert are the columns that are places: `node`'s start and end, `line`,
  `end_line`, `node_order` and `visit_index`. The rows gone and added have the same ids, and differ only there.
- So the cut is per atom, by the columns it reads ("Built", B, `Net`): `node(d, v, _, _, _)` saw no change.

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

## Built (2026-11-21)

### A1 and A2

- **The predicate** (`run_observed_within`, after the split): the file-local part answers no request (`!parsing`),
  the run is not `.embedded on`, and no document may embed a program (`may_embed`). A document may not where:
  - its language embeds none (`Dialect::embeds()` empty);
  - or the store records its text, in its grammar, grammar version and tape format, as loaded with no programs
    (`ContentStore::programs_counts`, the `tape_objects.programs` column, 0 = none, below);
  - or the store does not know its text (a new file, an edit, an empty store), its language embeds only by call
    argument (Python: `Markup`, `format_html`, 0470), and the text names none of the places' callees. That is the
    embedder's own test (`embed::call_argument`'s `memchr` precheck), which then builds nothing. The text is read
    as the run reads it: held, else the file.

  Any other embedding (an HTML file's scripts, a Vue file's blocks) is of the file's structure, so such a document
  may embed unless the store says it holds none. There is no switch: the pruned parts are a function of the split's,
  whose digests the memo's and the components' keys are, and a document's text and grammar are in those digests.
- **The store records "no programs".** A load wrote a tape's program count only where it was not zero, so a count
  of 0 read the same as unknown (−1). `ObjectsWrite::run` now writes 0 for a load with none
  (`ContentStore::put_no_programs`, only over −1). `attach_programs` already read 0 as "recorded, none".
- **The guest-only set** is `guests::ONLY`, beside `READS`.
- **One pass, `prune::Unfillable`**, forward and backward:
  - forward: the closure seeded from the empty inputs;
  - backward: from what the dropped rules read alone, the relations no rule left reads, never an output, a native's
    or a request's.
  - A part's exports are kept: the general `Prune` took one of them from the file-local part, because its roots are
    the outputs alone. That broke `comments_are_ws_comment_nodes` (a Rust batch). So the general pass does not run
    on a part.
- **Tests:** `prune::tests::a_rule_joining_an_empty_input_goes_with_what_only_it_reads` (a rule over an empty input
  goes, with the demand only it read; a negated empty relation keeps its rule; a relation a phase binds beside its
  rules is never taken empty), and the guest tests of `corpus` and `rr`.
- **Test:** `session::tests::a_session_drops_the_guest_rules_where_no_document_embeds`. A Python batch whose
  `parsed` is asked only across files: the first run (an empty store, the callee test) and an edit (one text
  recorded with none, one new text naming no callee) drop rules (`Report::unfillable`); an edit that adds a
  `Markup("…")` call keeps them. Each run's findings are a full run's.
- **The pass is worklists**, each rule and read visited once. As fixed-point sweeps over ep2's 15,000 rules it took
  1.38 s of an edit's 4.5 s; as worklists, 11 ms.
- **ep2**, M3: A fires on the cold run (an empty store) and on every edit; none of its 300 files names `Markup` or
  `format_html`. Cold: 8,097 → 5,655 strata (the spike measured A at about 2% of a cold run's instructions); the
  store then records 618 texts with no programs. The edits, with A on, are in B's second table below.

### B: the reach by kind and key

At the driver (`plan.rs`, `Reach::cuts`, where `moved` is formed), before a stratum's spec and plan. A stratum none
of whose rules sees a row of its changed reads' deltas takes the quiet path. It is counted in `Outcome::unseen` and
`Report::cross_unseen`, with its own profile line. `RERAIL_REACH_BY_KEY=off` reaches by relation alone, to compare.
Per atom over a changed read:

1. **The atom's own tests** over the delta's rows, read where they lie: a kind's constant, a kind set's facts, a
   field's or symbol's constant. No delta row passes them, so the atom sees none.
2. **A witness that shares a variable:** another positive atom, outside the stratum, over a relation this program
   derives. No delta row's value in the shared column is one of its rows' under its own tests.
   - It is read through its own index: `lookup_read`, `sorted_rows`, else `lookup`.
   - These are its rows now. Where it changed too, they are its rows gone as well.
   - A stored relation not read yet is bound where it lies (`Maintain::need`, 0401 F3) and then probed.
3. **Only the rows that count, by the columns read (`Net`).** A row gone counts only where no row added holds the
   same values in the columns the atom reads, and the other way round. A column is read when it holds a constant, or
   a variable used elsewhere in the rule (`var_uses`; with an aggregate, a function term or a nearest-ancestor or
   AST literal, every column but a `_`). An insert's rows past it differ only in places, so to `exists_N(d) :-
   pN_X(v), node(d, v, _, _, _)` nothing changed.
   - Each row's read columns are looked up in the other side through its own index. The column looked up first is
     the one that index finds fewest rows for (`estimated_matches`). Keyed on the document, every lookup returned the
     edited file's 4,341 rows, 18.8 M comparisons and 290 ms.
   - The rows that count are named as an input's delta is, by their places (`InRel::picked`, 0364 R2). They are made
     once a run for each relation and set of columns read (`NetDeltas`), because a check per row in each stratum's
     walks cost more than the cut saved.
   - The test's third edit moves a.py's loop a line down. `exists_0`, which joins `node` to the findings, is cut, and
     `violation`, which reads the loop's line, is not. With `Net` off, the test fails there.
4. **Read first, the delta**, for as many rows as the witness has. A reached stratum finds its match in the first
   rows. Then, where the witness is the smaller, its rows are walked and each is looked up in the delta's own index.
   So a probe reads at most twice the smaller side. That took the insert's probes from 364 ms to 47 ms.

Not cut:

- a native's stratum;
- a view of an input;
- an aggregate over a changed read;
- a nearest-ancestor or AST literal;
- a changed read no rule names as an atom.

**Never joined as a witness: an input read through a view of the batch's documents**, and a changed input's rows
now. A lookup there builds the documents' rows. On ep2's insert, one probe of `docof` built 295 documents' rows and
AST matchers, about 420 ms, which a run that keeps its strata never builds. A relation held as kept counts as an
input of the maintained program (`Fields::is_input`) but is read as kept rows, so the test is "derived here, held
or kept", not `is_input`.

**Unlike the design**, the delta's kinds and keys are not facts written with the delta, and stored relations have no
key summary. The delta's rows are read in place per atom, and a stored witness is bound where it lies. The probe
costs 29–47 ms over a whole edit, so neither was needed.

**Test:** `session::tests::a_session_passes_on_a_stratum_its_changed_kinds_do_not_reach`. An assignment added to a
file cuts the strata reached through `kind` and `node` (`exists_0` among them), and a `while` loop added after is
seen. A line put at a file's top cuts `exists_0` but not `violation` (above). Each run's findings are a full run's.
`Report::cross_unseen` names the strata cut.

**Measured on ep2, M3, unprofiled.** Two stores, B off (a) and on (b), sides alternating on each edit. The outputs
(`outputs.rro`) are identical on every run.

| Edit | Instructions | CPU | Wall | Peak memory |
|---|---|---|---|---|
| `append` (a function at a file's end) | 60.1 → 37.8 G | 4.79 → 3.29 s | 3.40 → 2.22 s | 855 → 675 MB |
| An import at a file's top | 66.1 → 61.4 G | 4.98 → 4.49 s | 3.70 → 3.43 s | 878 → 783 MB |
| A comprehension added at another file's end | 61.8 → 58.5 G | 4.85 → 4.69 s | 3.54 → 3.34 s | 921 → 903 MB |

With A firing too (both sides; B off and on, the same machine, a later run):

| Edit | Instructions | CPU | Wall | Peak memory |
|---|---|---|---|---|
| `append` | 59.1 → 36.6 G | 4.48 → 2.97 s | 3.21 → 2.05 s | 855 → 648 MB |
| An import at a file's top | 64.7 → 60.0 G | 4.50 → 4.07 s | 3.40 → 3.18 s | 878 → 766 MB |
| A comprehension at another file's end | 61.0 → 57.4 G | 4.53 → 4.31 s | 3.31 → 3.10 s | 1,018 → 862 MB |

- Before `Net`, the insert was even (66.0 → 66.6 G): 363 strata were cut. With `Net`, 444 are, at 124 ms of
  probes.
- The profiled edit runs overstate the edits by 20–40%, so the numbers above are unprofiled.

### After A and B: the edits' profiles

With A and B built, the insert and the comprehension edit gained little (6–7%), so both were profiled (M3, A and B
on). Their join steps cost 31 ms and 19 ms. Their cross-file joins cost 0.96 s and 1.41 s, and most of that was one
removal join each building every document's rows (decision 0494), for an edit to one file:

- **The insert: `docof` read as it was.** A removal join reads a changed input as it was (0364). `docof`'s old
  version, probed by a node, made the node's document. `rewritable` (`upgrade-typing.rerail`), from
  `future_annotations`'s gone row, walks `typing_ref` and tests each node's document, so it made 121 documents,
  0.74 s. The node's document is its id's (0083 §3), so with the node bound the join now computes it (`DocOf`). The
  id's document holds of every id the old rows hold, so the join finds at least what it did; a removal's rows are
  candidates, each rechecked against the rows now. Test:
  `made_batch_tests::a_removal_join_computes_docof_from_the_node`.
- **The comprehension edit: the join screen measured its probe side first.** The Bloom screen (0488) measured the
  probe side's rows before it checked the scan's against its floor (256). A removal join of a few `text` rows, then
  a text test (`strtest_51`), so asked the text test's length. Unknown after an edit (one document's counts are
  not given), that is every string that passes, every document made: 299 documents, 0.98 s, for a walk that is never
  screened. The scan's floor is now checked first. Test: `join_filter::tests::a_small_scan_does_not_measure_its_probe_side`.

ep2, M3, unprofiled, A and B on; outputs identical on every run:

| Edit | Instructions | Wall | Peak memory |
|---|---|---|---|
| An import at a file's top | 60.0 → 50.2 G | 3.18 → 2.70 s | 766 → 705 MB |
| A comprehension at another file's end | 57.4 → 39.0 G | 3.08 → 2.20 s | 862 → 753 MB |

The append is unchanged (36.6 → 36.8 G). With B off, the append fell too (59.1 → 39.3 G): the screen's measure was
much of what B cut there.

### The import's cross part, profiled again (2026-11-22)

After the two fixes the import's cross part is the dearest phase left of the three edits (1.04–1.10 s on the M3,
0.51–0.62 s for the others). An import at a file's top moves every line of that file: `line`'s delta is 9,350 rows,
`node_order`'s 8,675. Five strata are derived again for it, by 0090 §4's priced clause (`RERAIL_MODE_TRACE`); the
dearest, `violation`, took a third of the cross part in the profile (0.57 s of 1.67 s on the mini). Derived again,
its 163 rules that read a finding's place (`p49_ASYNC100($v0, _), node($d, $v0, …), doc($d, $p, _), line($v0, $l),
col($v0, $c)`) probe `line` by each finding's node, and a probe makes the node's document (0494): 33 documents
(`line column 0 ×33`), for an edit to one file.

Maintaining it instead cost 34.8 s (`violation` alone forced into the mode; outputs identical). Each rule's removal
and add lines led with `line`'s delta and read the finding last: `line → node → doc → !scope_drop → col →
p49_ASYNC100`. The finding is one row, its rows measured, its columns' distinct values not; the containment share
(the probes that hit) is skipped where a column's distinct values are not measured, so the probe was priced as
keeping every binding. A column holds no more distinct values than its relation's rows, so where its own are not
measured, the rows measured bound them (`plan.rs`, a relation of the component being solved excepted, its rows still
growing). The lines now read `line → p49_ASYNC100 → …` (each priced 4,870, against 23,537), and `violation`
maintained costs under 0.02 s of a 0.83 s cross part (profiled). Test:
`plan::tests::a_probe_of_a_relation_measured_by_rows_alone_prices_its_misses`, which fails without it.

ep2, M3, unprofiled, A and B on, every edit's outputs the rows of a fresh cold run of the same tree (all 661
relations):

| Edit | Instructions before | after |
|---|---|---|
| The append | 36.8 G | 36.6 G |
| An import at a file's top | 50.2 G | 48.6 G |
| A comprehension at another file's end | 39.0 G | 38.6 G |

The priced clause still derived the five again: the mode at 1.9 M, the derive at 3,501. With the clause off
(`RERAIL_AGAIN_PRICE=1e15`) the import's cross part was 0.75 s against 1.05 s, outputs identical. The derive's
price is its joins' rows, but deriving `violation` again also made the 33 documents, 0.19 s of its 0.41 s
(`MadeRows::routed`, sampled). The mode's price was its lines' real order.

**A maintain mode's line starts where it is cheapest.** A removal or add line is a whole join (`Line::Remove`,
`Line::Seed`), and a changed input's version is probed through its own index wherever it is read
(`Source::Version`). Only a semi-naive probe's delta must lead its wave. A positive driver's line whose relation's
change is past 0090 §4's floor (`RERAIL_AGAIN_MIN`) is now planned with its delta sized by its rows but free to start
anywhere (`Stats::lead_free`, its plans keyed apart). Probed through a bound variable, the delta is priced as the
relation's probe at the delta's share of its rows. Each of the 163 lines reads the finding first and probes the
moved lines by its node. Under the floor a line costs little whatever its order, and keeps the program's: planned
free, its prepared plan was not taken, and each append cost 1 G more (38.0 against 37.1 G). A negated driver still
leads its line (`Body::with_bound`).

**A line priced over its rule's derive is searched before the comparison.** A kept order is run as it is while its
work re-priced is under `REPLAN_ROWS` (10,000): a search could save no more than that. The price clause sums 490
such prices against the derive. After the append, three lines kept from its few moved lines (led by `line`'s delta
there) were each priced 9,836 at the import, against a best of 10 to 60: the mode at 30,377, `violation` derived
again. In `mode_costs`, a line priced over its rule's derive is searched, and priced at the cheaper of the two.

With both, `violation` is maintained: mode 1,039 against derive 3,505. ep2 on the M3, unprofiled, A and B on, cold
then the append, the import and a second append, each side its own store, sides alternating; before is the same
build with neither (outputs identical between the sides on every edit; without the floor, the last edit's rows
equal a fresh cold run's in all 661 relations):

| Edit | cross before | after | Instructions before | after |
|---|---|---|---|---|
| The append | 0.53 s | 0.52 s | 36.8 G | 36.7 G |
| An import at a file's top | 1.00 s | 0.68 s | 48.8 G | 42.6 G |
| A second append | 0.61 s | 0.61 s | 39.0 G | 38.6 G |

The import's cross part is now within 0.1 s of the appends'. Tests:
`plan::tests::a_maintain_mode_line_starts_where_it_is_cheapest` (fails with the start fixed);
`maintain_tests::a_finding_s_places_moved_are_maintained` (cold, an append past the floor whose line its few new
places lead, then an import at the file's top; outputs a fresh run's; fails without either change: mode 526
against derive 404).

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
3. ~~**Stable node identity** across an edit.~~ It was already built: ids are by definition (`granules`). An
   insert's delta is the places that moved, and B's `Net` reads past them.
4. **A3**, on a corpus whose file-local part answers requests: the guest inputs counted empty once admission has
   closed. A Python batch like ep2 no longer needs it: the store's program counts and the callee test decide before
   the split (A1–A2, above).

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
