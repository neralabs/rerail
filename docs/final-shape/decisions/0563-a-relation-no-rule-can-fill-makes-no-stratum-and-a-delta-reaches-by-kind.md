# 0563: A relation no rule can fill makes no stratum, and an edit's delta reaches a rule by the kinds it holds

Status: designed (2026-11-21), not built. The user asked for it (2026-11-21): "okay, please survey and design this.
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
