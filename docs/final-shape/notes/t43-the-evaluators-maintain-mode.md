# t43: The evaluator's maintain mode (0364 stages 2–6), a working plan

## Intro

Decision 0364 replaces the maintained forms (rules over `h__old`, `h__del`, `h__redo`, `h__ins`, `h__minus`,
`h__plus`, and for levels `h__cand`, `h__chk`, `h__held`, `h__gone`, `h__live`) with a mode of the native
evaluator. That mode reads one relation in its versions and keeps a level per row of a recursive relation. This
note is the plan of that work, kept current as each stage lands. 0364's Implementation section is the decision;
this note is how it is built.

## What the engine has (2026-11-17, at b674d923a)

- **A maintained relation already starts from its kept one in place.** `Rel::based` (setfix.rs) shares the kept
  relation's rows and orders, marks `gone`'s rows dead, and records `Origin::Based { at, n }` (its row count then).
  `stats_dead` is its dead bits then, but the statistics move it on (`count_dead`), so it is not a version.
- **Deltas are copied out.** `InRel::delta_since` and `delta_since_mark` return `(added, removed)` as row-major
  `Vec<i64>`; the frames' deltas and the kept statistics' patch read them (retain.rs).
- **The forms make the deltas as relations.** `maintain.rs` writes, per stratum reached, `h__del`, `h__redo`,
  `h__ins` and the net deltas `h__minus :- h__del, !h__redo, !h__ins` and `h__plus :- h__ins, !h__old`. Those are
  planned, placed and run as components of their own (plan.rs, `m.form`, `run.add_form`).
- **`h__old` is a source** the driver hands the run (`Maintained::sources`), adopted in place.
- **Inputs' deltas** come from the layered sources (`input_delta.rs`): `r__minus`, `r__plus` for each changed input.

## The stages

### Stage 2: one relation, versioned

- `Rel` records its version at the run's start (`start: n0` and the dead bits then), set by `based`; the finished
  `InRel` keeps it (`Version0`).
- Views of it, read in place, none a copy of rows:
  - *old*: the rows `0..n0` with the start's dead bits: an `InRel` sharing the rows (`Rows::shared(n0)`);
  - *new*: the relation itself;
  - *minus*: the ids `< n0` live at the start and dead now: a view of ids over the relation (`View::ids`);
  - *plus*: the ids `≥ n0` live now: a view of ids.
- `delta_since` reads the version, not the relation it started as.
- Tests: the views of a relation that started from a kept one, with rows killed and added; a relation with no
  version has no *old*, *minus* or *plus*.

### Stage 3: the maintain mode

Per component reached, from its own rules and the changed atoms of each:
- removal joins: per rule and changed atom, the atom's *minus* over the other atoms' *old* gives candidates;
- add joins: the atom's *plus* over *new* gives rows to add;
- a non-recursive component rechecks each candidate by its rules with the head bound over *new*; one with no
  derivation left is killed in place;
- a recursive one takes candidates in level order (0364 decision 2), then levels the rows gone for now and the rows
  added;
- candidates and rows to add are buffers of the run, not relations.

### Stage 4: plans with the program

A rule's plan per driving atom, and its recheck plan, made with the program as the semi-naive delta plans are. A run
selects those of the atoms that changed.

### Stage 5: relations kept in place

A session's relations stay in their blocks between runs; nothing moves to frames and back.

### Stage 6: the forms go

`maintain.rs`'s writer, general forms and `StoredForms`, `Maint`/`AgainRef`, the levels lines, and the `__old`,
`__minus` and `__plus` sources.

## Progress

- Stage 1 (counting): done, 011fc99ef.
- Roll-up of the related worktrees before stage 2: done, b674d923a (5 merges).
- Stage 2: done, db49bbdb9. `Origin::Based` carries the start's dead rows; `InRel::start`, `version_ids`,
  `version_view(Ver::{Old, Minus, Plus})`; `delta_since` reads the version. A join step does not yet name a
  version: that comes with stage 3's mode, which is the first reader.
- Found on the way: the version's prune lacked the tree's revival of a pruned on-request attribute as an input
  (44dfb61af); every session test had stopped on it.
- Stage 3: done.
  - b3d2c3419, the engine: `Source::Version` and the `remove`, `addchecked` and `recheck` lines (`MaintMode`).
  - f116df144, non-recursive strata: the driver gives a reached stratum the mode, with no form, where it fits
    (`Run::maintain_mode_blocks`). A changed input's versions are its sources, held while the stratum runs
    (`r__mm_*`, `Run::mode_reads`).
  - 211a599bb, recursive strata by levels (0364 §2):
    - candidates in level order; a recheck sees only rows below the candidate's level (`Rel::cap`);
    - a row gone makes its readers candidates, through each delta probe run from that one row (`drivers`);
    - rows gone that a rule derives now come back as new rows, and they and the checked adds are the first wave's
      delta;
    - a view of an input's columns has its input's versions, through its columns.
  - In the session tests, 927 reached strata take the mode. The rest read a relation with no version (a native
    operator's, one derived again), or keep their rows on disk only.
  - `a_cycle_closed_by_an_empty_relation_still_keeps_levels` passes now.
  - Still on forms:
    - strata over a relation without a version;
    - aggregates, `nearest` and `ast` literals;
    - lattices, gates and requests;
    - strata whose kept rows are not in memory.
    0090 §4's "derive again" choice is not made for a mode stratum yet.
- Stage 4: the mode's plans are the program's.
  - Its removal, add and probe lines take the kept plans (`plan_rule_reused`), as the semi-naive probes do: searched
    at first sight, then taken or priced again as their sizes move.
  - A quiet recursive stratum plans no probes.
  - The recheck keeps the greedy bound-head order: no planner prices a bound head yet (Q208), and the lines report no
    work to the 0122 budget (Q208).
- e1861f68b, stage 4.
- Stage 5: relations kept in place between runs, surveyed; the mode widened.
  - A session already keeps its part's relations in memory between runs (`Retained`, the session's cell). The mode
    starts each from its kept rows in place (`Rel::based`), and its result is the next run's kept relation.
  - The 459 "kept rows not in memory" fallbacks were all of a part's first run, which has nothing kept and no frames.
    They are now named so ("nothing kept: its part's first run"), and are derived as before. A process with no
    session reads its part from the frames, by design (K4).
  - A relation a form maintained has no version, but its form derives the net deltas against the kept rows
    (`r__minus`, `r__plus`). Its readers now take those, with its kept rows as old, as they take a changed input's
    versions. Only deltas this run made count (`has_delta`), because the engine may still hold an earlier run's.
    This took the session tests from 927 strata in the mode to 1095.
  - Found and fixed on the way: a recheck took every rule of the head with the head's slots preset, but never
    compared the head's constants. So `v("SEESE", i) :- …` "derived" the candidate `v("CYC", i)`, and it stayed.
    That gave wrong findings in `the_reach_index_is_derived_again_when_cycles_and_connections_change_in_a_new_process`.
    `MaintMode::derives` now skips a rule whose head constants (or repeated slots) cannot name the row. Unit test:
    `a_recheck_takes_no_rule_whose_head_constants_differ_from_the_candidate`.
  - Still derived again: the reach index (decision 0119) and its readers, as its tests expect.
  - Still on forms: literals other than atoms and comparisons (30 in the session tests).
- 8e64f3fbb, stage 5.
- The mode widened before the forms go (the user, 2026-11-17: "widen, then delete"):
  - Aggregates (`count`, `sum`, `min`, `max`, `join`) over a changed lower relation, by their groups, as the forms'
    `__grp` relations did. Per minus and plus of the relation, there is a removal join (the rule over what was, its
    aggregate over the old rows) and an add join (the rule now). Each is led by an atom over that delta, holding the
    aggregate's group slots (its bound arguments) with its own columns wild. Aggregate items read the versions by
    `mode_reads` as atoms do.
  - Heads with computed terms: a recheck presets only the head's plain slots. It now compares each row a rule
    derives with the candidate, and never takes a row of the bound columns alone.
  - Test: `the_maintain_mode_keeps_aggregates_by_their_groups_and_computed_heads`, three edits, each against a run
    with an empty store.
  - Still not in the mode, and derived again once the forms go:
    - lattice, gate, request, native, produced, fact and input relations;
    - nearest and ast literals (no native step);
    - aggregates over a changed input or input view (read projected; its versions are whole), gated, or over their
      own stratum;
    - recursive strata whose kept rows have no levels;
    - readers of the reach index (derived again by design).
- Stage 6: the forms are gone.
  - ebd966e73 (6a): a stratum the mode does not take is derived again, with no form. Its delta is its diff against
    its kept rows (`kept_delta`); a relation derived again that did not change is not counted as moved.
  - 0098f4340 (6b): a part the level driver does not walk (no kept part, or gates) derives its cut's strata again.
  - 6c, in two commits:
    - The writer: maintain.rs went from 2,002 lines to what a run reports (`Maintained`, `levels_wanted`, the
      levels counter). Also gone: `Slot::Reached`, the form jobs, `general_selection`, the `Driver`'s form fields,
      `RERAIL_SESSION_VIEWS`, and the O125 counts of `__del`, `__ins` and `__cand` rows.
    - The forms' layer in the program: `body::Maint`, `AgainRef`, `Group`, `Placed.maint`, `Versions`' again
      list, the AST's `Program::again` and `together` and `front::Again`, the `together` rings of the components
      search (`front::apart`), `Builder::of_program_less_maintenance`, and in the driver 0090 §4's choice between
      maintaining and deriving again, with its deferred deltas (`RERAIL_AGAIN_DEFER`, `RERAIL_AGAIN_THETA`, …).
      Also gone: the form strands (`strand_forms`, `run_forms`, `absorb_forms`), `add_form`, `add_again` and
      `spike_delta`. A maintained run (`maintaining`) still keeps its seed joins out of the level batch, as its
      groups did.
  - Red throughout, as before this work: Q80, Q206 and Q207. Q207 also covers two rr tests first run here.

## Review against the design (2026-11-18)

The user asked for a step back: stages 2–6 departed from what 0328 and 0364 agreed. Their required shape is now
0364's § "Required shape" (R1–R10), with a test each in `plan/corpus/maintain_tests.rs`. Where stage 6 stood:

| | Met? | What departs |
|---|---|---|
| R1 | no | The mode's relations hold their versions as `r__minus`/`r__plus`, a changed input's as `r__mm_old`/`r__mm_minus`/`r__mm_plus` (44 a small edit, in the test), and the run path still names the forms' suffixes. |
| R2 | no | `kept_delta` copies and sorts a stratum derived again and its kept rows; an input's change is copied from its documents' entries (`input_delta::delta`); `version_delta` and `delta_since` copy the rows of a change for the frames and the statistics. |
| R3 | no | The mode refuses many kinds, each then derived again: aggregates over a changed input (on every small edit of the test), lattices, gates, requests, native operators, `nearest`/`ast`, reads changed without a version. |
| R4 | no | 0090 §4's threshold went with the forms in 6c. |
| R5 | yes | The ring's rows all go, maintained by levels. Levels are a `Vec<u32>` beside the rows and a frame part of their own, 4 bytes a row as decided. |
| R6 | no | The static check is a script (`wf.py`). |
| R7 | no | The recheck takes one greedy pass, unpriced (6 lines a small edit). |
| R8 | yes | A session's edit reads no kept frame. |
| R9 | no | The forms' driver (cut, slots, `Driver`, `maintained.rs`'s slice) still decides; the report's derived-again count and reasons disagree. |
| R10 | yes | On the tests' edits. |

## The plan (2026-11-18)

Each step lands with its requirement's test un-ignored.

1. Measure the O125 replay: instructions, facts held and kept bytes per edit, each edit's outputs against a fresh
   run. Which departure costs the most, and the goal's figures against the forms'.
2. R4: the threshold again, in the mode's terms.
3. R1, R2: a relation's change read off its version in place, for inputs, mode strata and strata derived again.
4. R3: count the strata derived again by reason, then bring the most common kinds into the mode.
5. R9: the driver down to skip, mode, or derive again.
6. R6 (the static check in the engine), R7 (Q208), R1's leftovers; the data-layer scale run last.

## Measurements (step 1, 2026-11-18)

`scripts/profiling/o125/maintain-measure.sh` over the O125 replay (77 files, `calls.rr`, steps 00–08), debug
build, `RERAIL_DEVICE=cpu`, on a farm host (M3 and M1 Max, otherwise idle):
- `cold`: a process per step over an empty store;
- `kept`: a process per step, its store kept (the CLI's path, the kept part from frames);
- `session`: one process (`repeat`, `RERAIL_SESSION`).

`maintain-report.py` makes the table. Its baseline is the forms' runs of 2026-11-17 (`tmp/o125/logs`, the same
harness, `dump.sh`): `qd` (DRed forms) and `kd` (counting).

**Correction (2026-11-19, step 18):** the forms' runs of steps 05-08 all panicked ("index out of bounds",
setfix.rs:4000, "host stream strata: a job panicked") and wrote no findings. Their figures below are the cost of a
crashed run. The baseline's fractions are of step 00's cold run (289 G); this note's are of each step's own cold run.
Steps 01-04 of the forms are correct (their findings equal this branch's cold runs) and are the only valid
comparison.

Instructions retired, as a fraction of the same series' cold run:

| Step | qd (forms, DRed) | kd (counting) | kept, now |
|---|---|---|---|
| 01 | 0.64 | 0.64 | 0.73 |
| 02 | 0.58 | 0.58 | 0.75 |
| 03 | 0.74 | 0.74 | 1.33 |
| 04 | 0.55 | 0.54 | 0.63 |
| 05 | 0.59 | 0.82 | 2.02 |
| 06 | 0.63 | 0.64 | 0.70 |
| 07 | 0.61 | 0.61 | 0.61 |
| 08 | 0.71 | 0.71 | 1.44 |

Cold is 322–339 G instructions now, against 289–310 G then. Each kept edit's findings are the cold run's (R10, 9
of 9). The session's edits took 13.5–32 s CPU each, against about 12 s CPU for a cold run, and the run stopped at
its 110 s cap after step 04.

**The goal is not met.** The kept part holds 1.0 M rows: 19.4–20.7 MB of values, 4.7–5.4 MB of levels and
6.4–7.3 MB of sets and indexes. A kept edit reads back 2,500–5,000 frame parts. An edit holds 800–1,400
maintenance relations and copies 15,000–110,000 rows to state its changes. 430–500 recheck lines go unpriced.

**What costs the most** (`RERAIL_PROFILE` of kept step 01, 16.5 s CPU in the cross part):
- 10.1 s (8.2 s of its own) goes to the mode's removals and rechecks, over 92 strata. All the run's native join work
  is 0.72 s.
- The recheck (`MaintMode::derives`) runs a rule's whole join with the head's slots preset as keys. That join is
  keyed (`join_rule_bound` makes a bound head slot a key). But the mode's joins never asked the component's relations
  for their orders: only the probes' joins call `Rel::want`. And the kept rows the store is given are in no order
  until the first wave's `set_delta`, which comes after the mode. So each recheck read every row of the component
  relation it probed: `best_order` found no order and fell back to the whole range, and `Cands::Pending` added every
  row not yet indexed. A stack sample (`SAMPLE_STEP=01`) showed this directly: the time sat in the walk's per-row
  dead and value tests.
- **Fixed** (2026-11-18): the mode's removal, add, recheck and readers joins ask for their orders when the component
  opens, and `MaintMode::run` first indexes the kept rows as rows of the wave before the first
  (`Rel::index_kept`). Step 01's mode went from 5.8 s to 2.3 s, and the 196 rows gone of one stratum from 1.14 s to
  0.03 s. The step went from 216 G to 186 G instructions, 0.58 of cold, against the forms' 0.64.
- Every row an add join gives is rechecked too (`derives` in the add loops), though the add join over the new state
  is itself a derivation. Only an add driven by a negated atom needs a check (0328's checked add).
- After that: the changed inputs' deltas from the documents' entries (1.2 s, `input delta by document`), and the kept
  rows from the frames (0.9 s).

**After the index fix, the threshold and its readers' fix** (79196a432, M3 Max, `RERAIL_JOURNAL=off`: Q189). Every
step's findings equal cold, kept and session alike:

| Step | Cold (G instr.) | Kept (G instr.) | Kept / cold | qd (forms) | Session CPU s |
|---|---|---|---|---|---|
| 00 | 323 | 322 | 0.99 | — | 27.2 (its cold run) |
| 01 | 324 | 180 | 0.55 | 0.64 | 15.3 |
| 02 | 325 | 168 | 0.52 | 0.58 | 12.8 |
| 03 | 325 | 226 | 0.69 | 0.74 | 15.4 |
| 04 | 324 | 157 | 0.48 | 0.55 | 9.8 |
| 05 | 329 | 287 | 0.87 | 0.59 | 19.8 |
| 06 | 328 | 177 | 0.54 | 0.63 | 11.1 |
| 07 | 334 | 186 | 0.56 | 0.61 | 11.3 |
| 08 | 339 | 240 | 0.71 | 0.71 | 16.1 |

Every kept step but 05 is below the forms'. A session edit costs 0.36–0.73 of the session's own cold run. The
session's shape counters over the nine runs: 2.29 M rows copied, 7,350 maintenance relations held, 2,015 recheck
lines unpriced. Those are step 3's and Q208's to remove.

Two bugs turned up in the course of this:
- The threshold derives again strata whose aggregate readers are maintained (`type_of`, then `type_first`, a
  `min`). The atom-driven joins did not name an aggregate's atom's version, so it read `type_of`'s own, which a
  relation derived again does not have. Fixed: the aggregate's atom reads the version it is given, as the aggregate
  joins already did.
- On the M3, the journal's replay sometimes missed the edit made just before a run, and the whole-run cutoff
  answered the last step's findings. That is Q189, and the measurement runs without the journal until it is fixed.

So the recheck was where the cost was, from a missing index rather than from the design. With that fixed, the rest of
the plan's order stands: the copies (step 3), then the derive-again kinds (step 4).

## Step 3: a change is its version, read in place (R1, R2)

Every relation an edit changes now states its change the way a maintained one does, as rows its version names
where they lie. Nothing is copied to state a change, and nothing is held under a second name.

| Relation | Its change was | Now |
|---|---|---|
| Maintained by the mode | its marks, copied into `Delta` rows for the frames and the statistics | its marks, read in place (`DeltaIds`, `delta_each`) |
| Derived again past θ | the kept and new rows copied, sorted and diffed into `r__minus`/`r__plus` (`kept_delta`) | the new rows made a version of the kept relation (`InRel::rebased`): rows no longer derived marked dead, rows newly derived appended, each row at the new derive's level |
| A kept input (frames) | the changed owners' rows gained and lost, copied into sources | ids: those lost among the kept frames' rows, those gained among the layered input's appended rows (`InRel::picked`) |
| A session's input | each changed document's whole memo entry cloned, its rows copied into sets | the memo holds entries shared (`Arc<Entry>`), and a change names each row by its entry and place (`input_delta::Delta`) |

The names went the same way:
- The mode's stratum starts from its kept relation held under its own name, not `r__old`.
- A changed input is held under its own name while the stratum runs, as a relation whose versions are given
  (`with_versions`), so its joins read `Version(r, …)` as they read a derived relation's. That replaces
  `r__mm_old`, `r__mm_minus` and `r__mm_plus`.
- The given input deltas go to the mode through the driver (`Maintain::given`). They are no longer declared as the
  program's inputs `r__plus` and `r__minus`, nor bound by name.

`r1_r2_an_edit_holds_no_maintenance_relation_and_copies_no_rows` runs again and passes. Still copying: a relation
derived again that cannot be made a version of its kept rows (no orders to start from, or a different width). It
keeps `kept_delta`'s diff, `Delta::Rows`, and that path is counted in the shape.

Two session tests set θ = 0 and no floor, and then asserted that a stratum was maintained. They passed only while
the threshold was gone. Under R4 they now assert what those settings mean:
- `a_session_derives_again_the_strata_its_deltas_outgrow`: every reached stratum is derived again, none maintained,
  and the findings are a full run's.
- `a_session_derives_again_past_theta_and_maintains_below_it` (it was `…only_the_strata_priced_cheaper_so`): at θ
  with no floor, some strata are derived again and some maintained in one session. The forms' pricing it once
  named went with the forms.

Over the O125 replay (M3 Max, `RERAIL_JOURNAL=off`, before the last fix below), every step's findings equal cold,
kept and session alike. Kept edits cost 162–294 G instructions against cold's 323–339 G, as before step 3. They
copy no row and hold no maintenance relation (before: 485–758 held an edit). Over its nine runs the session holds 16
(before: 4,113) and copies 102 K rows (before: 2.29 M). Those copies are its outputs' derives again: `Rel::based`
refused them when an order covered fewer rows than the relation, or when more than an eighth of its rows were
dead. Now `based` starts from the orders that cover every row, and `rebased` compacts a kept relation that has
too many rows dead and makes the change marks against its live rows. A unit test covers it
(`a_relation_derived_again_is_a_version_of_its_kept_rows`); the replay has not been run since.

A first form of the input versions replaced an input the engine holds with a view of it while its stratum ran, so
its probes lost its indexes (kept step 04: 231 G against 163 G). The versions now lie beside the relation
(`give_versions`), and its own rows are read as they are.

## Step 4: the kinds still derived again (R3)

Counted over the session replay, the strata derived again for a reason R3 does not allow:
- **`cparam` changed without a version** (28 over the session's nine steps): its readers (`tmap`, `tparg`, …) find
  `cparam` changed but holding no version. Why `cparam` itself has none is not known yet: on the corpus path the
  preparation pass makes `nearest(c, m, scope)` an input atom (`nearest_scope(c, m)`, `lower::transduce_nearest`),
  so `cparam` and `gbase_at` are plain joins, and the mode's refusal of a `nearest` literal never applies to them.
  An earlier reading here blamed that refusal. The replay's trace (mm12, after step 3's compaction fix) shows
  `cparam` with no kept rows at every step: "nothing kept: its part's first run" on the steps whose reads change
  little, past θ on the others (3,922 delta rows against θ × 0 kept). So the run keeps no rows of `cparam` at all,
  and each run derives it again with no version. Why it is not kept is traced next (`RERAIL_SHAPE_TRACE not kept`).
- **An aggregate over a changed input** (every small edit of R3's test: `width` counts `child`).

**Aggregates over a changed input.** The refusal was there because an input's rows are read projected (its
query's columns) while its versions are whole. The add join is led by a delta atom that holds the aggregate's
arguments over whole columns, so the aggregate's own read must be whole too. An input's versions now include its
rows now (`New`, given beside it with `Old`, `Minus` and `Plus`), and the add join reads the aggregate's atom from
them. Both refusals ("…over a changed input", "…over a changed view of an input") are gone.

**`nearest` outside the corpus path.** A run that is not prepared (the plain `run`) keeps `nearest` literals, and
their spec item has no engine step, so such a stratum runs in SQL. A native climb step (the tape's parents, each
tested by its kind in `node`) is written and parked, uncompiled, in `working-data/native-nearest-step.patch`. The
corpus path does not need it, and it is not part of 0364.

## Step 5: the driver's three outcomes (R9)

Each stratum the walk reaches had one of five handlings, and only two were counted. The driver's `counts` and
`again` slots were left over from the forms and never written, so the report's "derived again" was 0 whatever ran.
- A mode run that changed nothing was counted nowhere.
- A stratum no changed read reaches, with no kept rows, was derived from its reads, uncounted.
- A native operator's stratum (the reach index) was made again, uncounted.
- The report's quiet count (`Outcome::quiet`, decision 0101 §1) counts the strata the mode took and found nothing to
  do in, not the strata the walk skipped.

Now the run's `Outcome` counts every outcome once: `walked` (reached, skipped, of all), `maintained` (and of them the
recursive), and `again`, each stratum with its reason (`NOT_KEPT`, `NATIVE_OPERATOR` added to `FIRST_RUN`,
`THRESHOLD`, …). The `Driver` keeps no counts, and the `walked` hook is gone. The report adds `cross_skipped`, which
R9 reads beside `cross_maintained` and `cross_again`.

## Step 6: the forms' leftovers go (R1)

What the forms left in the run path, with nothing left to make it:
- **Levels, survivors, `based`.** The native program spec's levels lines (`Line::Levels`, the `Levels` driver of
  setfix), the quiet survivors stage of the composed run, and the `Start::Base` seeds. The compose test that built a
  DRed program by hand to drive them (`quiet_composed_maintenance_keeps_the_same_resident_fact`) went with them.
- **The planner's maintenance names.** `changes_with_the_delta` (`__ins`, `__redo`, `__del`), `smaller_change` and the
  distinct-demand pricing (`__plus`, `__minus`), `reads_change` (ten suffixes), the `__old`/`__live` statistics
  fallbacks of `prior_stats`, and the unsized-delta counter (no relation is named `r__plus` any more). Each tested a
  name no relation has, so each was constantly false.
- **Statistics kept by delta** (decision 0096 step 4). They were made for a relation bound as `r__old` that a form
  maintained, and no relation is bound so: `bind_kept`'s `maintains` was false at every call. Gone: `Patched`, the
  cell's `patched` map, `bound_old`, the input statistics a new process gave `r__old` (`input_stats`,
  `projection_of`, `Stats::of_columns`), and the frames' statistics kind 0. The frames' statistics part is now
  format 2, measured statistics only. A format 1 part reads as none: the relation is measured again once.
- **`r__minus`/`r__plus` held for a stratum derived again.** Where a relation derived again cannot be made a version
  of its kept rows (a reach index held over its source, decision 0119, in a chunk's guest rounds, 0331),
  `kept_delta` held its diff under those names. Now it returns the rows gone and added, and its readers are given
  them as its versions (`diffs`), beside it, under no name. The diff still copies both sides' rows to compare them:
  0364 R2's one open case, counted by `rows_copied`.
- **The given deltas' column names** (`r__plus_c0`) are the relation's own: the planner prices a version by its
  relation (`delta_rows`), never by these names.
- **The spike's form counts** (`Outcome::spike_forms`, `spike_reached`), never filled once the forms went.

With these gone, `r1_the_engine_source_names_no_maintenance_relation` passes and is no longer ignored.

**A view of an input, reached.** A view of an input's columns (Q37) keeps nothing, so a reached one was taken for
its part's first run (`FIRST_RUN`, R3's allowed reason): r3's own trace showed `doc_tape`, `document` and
`library` "derived again" at every step. In the corpus, `cparam` (`cparam(c, i, y) :- fl…_cparam(c, i, y)`, a whole
copy of a file-local export) went past θ instead (its delta against 0 kept rows), and its readers then found no
version of it: `tparg`, `tvar_of`, `rtvar`, `ownvar` and the 44-relation stratum were derived again ("cparam changed
without a version"). Now a reached stratum shaped as a view (`view_shaped`, whether or not its input is bound yet;
a file-local export is bound when its first reader runs) is held again in place and counted maintained, the mode's
trivial case: its version is its input's, read through its columns. Its readers' `ins` gives them that: the input's
given versions, or, for an input the engine holds with its change marked in place (the export), the input's own
versions read through the view's columns. R3's test now allows only θ: each of its edits follows its part's first
run.

**A view by its program, not by a run's rows.** `cparam` still went past θ with 0 kept rows: it has two rules in the
cross part, the export's copy and a cross-file rule (`gbase_at`, …) over relations empty in these runs. The native
stratum leaves out a rule over an empty relation, and then took the copy rule left for a view of an input: held as a
view, so never kept, so derived again at every step with no version. Now a stratum is a view only when its program's
rules make it one (`written == 1`, before any is left out). `cparam` is derived, kept and maintained, and its
readers are maintained (O125 steps 00–02: no stratum "changed without a version", 186 maintained against 180).
R3's test holds the case: `defq` copies `def` and joins `lonely`, a cross-file relation that is always empty.
Without the check it fails ("not reached, its rows not kept").

## Step 7: the static check in the engine (R6)

`plan/wellfounded.rs` is `wf.py` ported (decision 0364 §5). It is an analysis of the whole program's version
(`Proofs`), made once, and a run asks it by component:
- the session's driver gives the one made of the program the parts were split from (`Programs::program`,
  `Maintain::proofs`);
- a run with no split (a plain run, a chunk's guest rounds) makes it of its own version.

It must read the whole program. In the cross part, a file-local helper (`attr_of`, `rasg`) is an input, and a join
pushed into the chunks (`fl<n>_<rule>`, decision 0051) holds tree steps whose rules the part does not have. Checked
over the cross part's version alone, every one of the corpus's nine maintained recursive components failed, `walk`,
`qn` and `icall/vpos/vroot` included, which `wf.py` proves. The check also reads `nearest` as the engine lowers it,
`rr_nearest_<set>(n, a)`, a step up.

A proved component:
- keeps no levels: neither its first derivation's `leveled` line nor the mode's;
- is maintained with no levels on its kept rows;
- gets a `measured` line in setfix in place of `leveled`. Its candidates go in a heap keyed by the measure, then the
  relation's rank: the node's depth (`ast_depth`) ascending or descending, its document order (`node_order`)
  descending, or the index;
- has each candidate rechecked once, over the relations as they are (no `cap`), and a row gone is never derived
  again: every row a candidate's derivations read is before it in that order, so already decided;
- is counted maintained, not by levels.

**Where the key cannot be read.** A part that holds no tree has neither `ast_depth` nor `node_order` bound: the
cross part of `calls.rr`, whose tree steps were pushed into the chunks (the replay's first measured run failed on
"no such table: main.ast_depth"). The measure's key there would be an input the part does not read, so the order is
dropped, not the proof. Unkeyed (`measured … u`), candidates go in id order, and a candidate that stays is queued
again when a row it may read goes. This is sound in any order:
- a kill is final: rows only go, so a row no rule derives over the rows now, none derives at the end;
- a stay is checked again whenever a support goes, and with no cycle that ends with every stay over rows that stay.

The key buys "once each". Without it, a row is checked once per support that goes, still with no levels.
Bringing the key to the cross part (its documents' depths, as an input of the part) was open; step 10 does it.

`r6_a_component_that_cannot_cycle_keeps_no_levels` passes and is no longer ignored. With the proof forced off it
fails (by levels: 1). Its first edit only adds rows, so it pushed no candidate and passed with a key no run could
read. It now also shrinks the body, removes the import and puts it back, each against a fresh run (R10) and with no
levels. R5's ring, which can cycle, is still maintained by levels.

## Step 8: the recheck's order is priced (R7, Q116)

The recheck took one greedy pass over its body with the head bound (`bound_head_order`), unpriced. Now it is
planned as the other lines are (`Model::plan_rule_bound`). The search starts from one row with the head's plain
variables bound, the state a recheck's candidate gives, and searches and prices orders from there. Such a plan is
no line's kept plan: only `plan_rule_reused` reads or keeps those, and a recheck does not call it. The greedy order
remains for a body none of whose priced orders has a native join, and is counted (`rechecks_unpriced`).
`r7_the_recheck_order_is_priced` passes over the small edits and is no longer ignored.

## Step 9: the data-layer scale run

The data layer's own history as a replay (`scripts/profiling/o125/dl-history.sh`): 00 is the `.py` tree of one
commit, and 01–03 are the next three commits' changes in `l2060_query_builder/`. The policy is
`conditional-architecture.rr` reading the whole tree. Each rung runs cold and as a session; "session = cold" means
the session's findings equal a cold run's at each step. CPU is per run, on an M3 farm host.

| Files | First run | Edits (01, 02, 03) | Session = cold |
|---|---|---|---|
| 37 | about 10 s | 1.6–4.7 s | yes |
| 334 | 65–72 s | 27, 24, 22 s | yes |
| 2,410 | 508–530 s | 477–579 s | yes |

At 2,410 files an edit cost as much as a first run: correct, with no saving. One edit at 334 files, profiled,
showed why.

**θ was measured against the wrong rows.** §4's share compared a stratum's changed reads' deltas with the
stratum's own kept rows. `rr_cover41` keeps 644 rows over `child`, `docof` and `col`, the whole tree. A 2-file edit
changes about 10,000 tree rows, so `rr_cover41` was derived again at every edit, along with 8 strata like it. That
cost was the corpus, not the edit. A derive reads its inputs whole and the mode reads their deltas, so the share
is now the deltas over the rows those reads hold. Where the mode's own checks still leave a stratum maintained and
the change is past the floor, §4's second clause decides: the mode's lines, priced by the planner (`mode_costs`),
against a derive's. These are kept plans that either path then takes as they are (`RERAIL_AGAIN_PRICE`, default 1).

**A negated changed read did not lead its line.** The mode planned a line driven by `!r`'s delta with no driver
(`first` was the positive drivers' alone), and `versioned` turned the atom positive at the place the plan had put
it as a test. The line walked its other atoms over the whole corpus to meet a delta of a few rows: 113 removal
joins took 10 s of the 31 s edit. Now the body is planned with the negated atom's variables bound
(`Body::with_bound`, the planner's state with variables bound before the first step), and the atom's item, now
reading the delta positively, goes first.

After both, at 334 files, the edits take 6.4, 9.9 and 8.4 s, and the session still equals cold. O125's steps 00–02
remain correct: 188 strata maintained, and 22 derived again where the mode was priced above a derive.

After the hub's merge (Q208's head-bound recheck, 0401's kept facts), on the same M3 host, sessions:

| Files | First run | Edits (01, 02, 03) | Shape |
|---|---|---|---|
| 334 | 65 s | 9.0, 13.2, 11.5 s | no row copied, no unled driver, no unkeyed measure |
| 2,410 | 436 s | 55, 65, 55 s | the same |

At 2,410 files the edits took 477–579 s at first. With the session memo fixed, they took 79–83 s. The memo now
keeps each file's current entry and the one before it (`HeldEntries`); its 4,096-entry cap had wiped it partway
through the session. After the merge they take 55–65 s. At 334 files the edits are slower than before the merge
(6.4, 9.9, 8.4 s); the cause is not yet known. A 2,410-file edit is still about 5× a 334-file edit, at 7× the
files.

## Step 10: the measure's key in the cross part (R6)

Step 7 left a gap. A proved component ordered by depth or document order reads `ast_depth` or `node_order` of
each candidate's node, and a cross part has no tree. There its candidates went unkeyed: sound, but a stay was
checked again each time one of its supports went, not once.

Now the split names, for each proved component of the cross part, the key relation its measure reads
(`measure_keys`). The chunks hand that relation on as an input of the part, the way they hand on a base relation
a cross-file rule looks a node up in: a semi-join, with each file's rows kept in its memo entry. The node set is
the cross part's own: the nodes its exports hold, and those of the base relations it holds whole. In `r6`,
`below(a, c) :- below(a, b), child(b, c, _)` takes `c` from `child`, so the exports alone would not cover it, and no
rule makes a node. This adds no copy: the key's rows are base rows handed on, like the part's other base inputs.

A removal's candidate can stand at a node the edit took out of the tree. That node has no key now, so the
candidate is *loose*: it goes first, and if it stays, it is checked again as a row it may read goes. If a loose row
goes, its readers are queued again, decided or not. This is the unkeyed rule from step 7, applied only to rows
with no key, and it is sound in any order. Every other candidate is checked once, in its measure's order.

`maintain::shape` counts a proved component run without its key (`measures_unkeyed`). The R6 test asserts none
in its cross part, through four edits, with each run's outputs equal to a fresh run's.

## Step 11: after the merge, O125 again

Two things stopped O125 on the merged engine.

**The hub's product-demand pass refused `calls.rr`.** `pv(o, "cls", c) :- attr_of(_, o, _), builtin_ref(o,
"object"), qv("builtins.object", "cls", c)` joins `o` and `c` through nothing. The pass pushed its demand up to
`base`, an output, and refused the whole program. That also broke the typing oracle test
(`python_calls_program_agrees_with_the_runtime`). By the user's choice, a node variable bound only by lookups on
constants is now fixed, as `c = constant` is. Its product is one side times that lookup's few rows.

**R6's key read was SQL inside the waves,** which `RERAIL_SQL_STRICT` refuses (decision 0104). It is now a lookup on
the key relation's bound source, in place.

O125 then ran, and step 02 copied 699 rows to state a change (R2). `cls_q` is a levelled stratum derived again past
θ: its kept rows had levels and the derive's rows had none, and `InRel::rebased` refused to start from rows whose
levels differed. It now takes either direction. Where the derive has no levels, the kept levels are dropped. Where
the kept rows had none, every live row is one the derive found or added, and it takes the derive's level. R5's test
now ends with an edit at θ 0 that derives the ring again, and asserts no row is copied.

| Step | Session CPU (M3) | Session = cold |
|---|---|---|
| 00 | 27.0 s | yes |
| 01 | 12.7 s | yes |
| 02 | 10.2 s | yes |

188 strata are maintained and 22 derived again past θ, as before the merge. Nothing is copied or held under a
maintenance name, every changed read leads its line, and every measure reads its key.

## Step 12: the goal, measured again

O125's replay, steps 00–08 (`calls.rr`, 77 files), on the M3 farm host, the harness of step 1 ("Measurements").
Kept and session findings equal cold at all 9 steps (R10).

Instructions per kept edit, as a fraction of the same step's cold run, against the forms' figures from step 1:

| Step | qd (forms, DRed) | kd (counting) | step 1 | now |
|---|---|---|---|---|
| 01 | 0.64 | 0.64 | 0.73 | 0.51 |
| 02 | 0.58 | 0.58 | 0.75 | 0.47 |
| 03 | 0.74 | 0.74 | 1.33 | 0.64 |
| 04 | 0.55 | 0.54 | 0.63 | 0.45 |
| 05 | 0.59 | 0.82 | 2.02 | 0.89 |
| 06 | 0.63 | 0.64 | 0.70 | 0.51 |
| 07 | 0.61 | 0.61 | 0.61 | 0.55 |
| 08 | 0.71 | 0.71 | 1.44 | 0.72 |

Cold runs take 361–380 G instructions. Kept edits beat both forms at 6 of 8 steps, and step 08 is level. Step 05
is short: 0.89 against 0.59.

What is kept, against step 1:

| | Step 1 | Now |
|---|---|---|
| Rows | 1.0 M | 649,000–689,000 |
| Values | 19.4–20.7 MB | 13.0–14.5 MB |
| Levels | 4.7–5.4 MB | 3.5–4.8 MB |
| Sets and indexes | 6.4–7.3 MB | 4.8–5.9 MB |
| Maintenance relations held per edit | 800–1,400 | 0 |
| Rows copied per edit | 15,000–110,000 | 0 |
| Unpriced recheck lines | 430–500 | 0 |

A kept edit, a new process, still reads 2,500–5,100 frame parts back, its kept part from the store. A session's
edit reads none (R8). In the session, the first run takes 28.6 s CPU and the edits 8.9–22.8 s, 0.31–0.80 of it.
Step 05's edit is the costliest in both kept and session.

## Step 13: a session's input deltas from the changed documents' entries

Profiling step 05's session edit (12.0 s of cross part) showed 2.81 s spent finding the inputs' deltas, and all
105 of them were found by hashing every document's rows, unchanged ones too. None were read from the changed
documents' memo entries. `input_delta::changed` looked up an entry under the whole program's plan digest
(`memo_whole`) and the run's absolute path. The memo holds an entry under its plan's id (`memo_plan.id`, the space
digest of 0225 §2) and the store path (`world::store_path`). So no lookup ever found one, and every session edit fell
back to hashing. Both keys are fixed. On O125, 111 inputs are now read from the entries and 4 are hashed. Reading
them still took 2.7 s for 180,000 rows, because `Delta` searched the entry's relations by name for every cell read.
Each entry's rows are now found once, and the read takes 0.88 s.

`maintain::shape` counts both paths (`inputs_from_entries`, `inputs_hashed`). R1/R2's test asserts that the
entries are read (0 before). The inputs still hashed are an open item of 0364.

Session CPU per step, after: 28.0 (00), 11.5, 9.8, 14.2, 8.6, 24.2, 9.8, 11.3, 15.8 s. Session = cold at every step.

## Step 14: a new process hashes only the changed documents' rows of its inputs

A kept CLI run (a new process) found each input's delta by hashing every row of every document
(`retain::document_delta`): 2.6-3.1 s of O125 step 05, and so of every step, however few documents changed. A
session reads the changed documents' memo entries (step 13). In a new process those entries are the store's: the
old content's entry is not admitted before the first wave, and resolving a content to its source is SQL inside the
waves (`memo.found`, refused by `RERAIL_SQL_STRICT`). So the store route was not taken. Instead:

- The documents a kept run read (path, key and content, as `input_delta::Docs`) are a frame of the part's space,
  `rerail:run_docs` under the cross program's `binds` (`retain::write_run_docs`). It is read with the cross part's
  frames before the first wave (`retain::run_docs_rel` among `frame_rels_kept`'s identities) and set on the kept part
  (`read_run_docs`, `Retained::set_docs`). It is written only when the list changed, not when it differs only in how
  a content is named (by digest, then by source once stored behind the run). A run whose inputs did not change writes
  no first record, so a warm run writes nothing.
- A document whose content and key are the last run's has its inputs' rows as they were, except in a semi-join's
  relation (`semijoin`, which follows other documents' exports). `retain::document_delta_except` takes those digests
  from the frame (`was`), counts each owner's rows (`documents_of`, falling back to the whole pass when an unchanged
  document's count differs), and hashes only the other owners' rows. A changed document's old rows are read by the
  kept input's ranges (`Retained::input_ranges`, when the source is the one adopted), and its new rows come from
  those already collected.

`maintain::shape::inputs_hashed_whole` counts the inputs whose every row was hashed. The new test
`r2_a_new_process_hashes_only_the_changed_documents_rows_of_its_inputs` (kept runs outside a session, with
`run_kept`) asserts it is 0 after each small edit. Without the frame write the test fails, with 4 inputs hashed whole.

O125 step 01 (a one-file edit, 75 of 77 documents unchanged), M1 Max: "input delta by document" 1.31 s, of which
hashing is 0.10 s. The rest is reading owners: 0.37 s counting, 0.44 s for the old rows (this input's old source was
decoded by `load`, so it has no ranges: see 0364's open item), and 0.19 s for the new rows. Step 05 (14 documents
changed, about half the lines) took 3.1 s before the other owners' rows were hashed apart. The kept route's other
open costs are 0364's: `load` copies and hashes the inputs it reads from the frames, and the kept-facts writer carries
a delta's entries forward (0353 row 12).

## Step 15: the goal measured again, and θ measured (0090 §4: "θ is set by measurement")

O125 00-08 on the M1 Max (the M3 was loaded at 24 and short of disk). Kept instructions as a fraction of cold,
with the forms' qd and kd beside them:

| Step | Kept / cold | qd | kd |
|---|---|---|---|
| 01 | 0.51 | 0.64 | 0.64 |
| 02 | 0.47 | 0.58 | 0.58 |
| 03 | 0.64 | 0.74 | 0.74 |
| 04 | 0.44 | 0.55 | 0.54 |
| 05 | 0.88 | 0.59 | 0.82 |
| 06 | 0.49 | 0.63 | 0.64 |
| 07 | 0.53 | 0.61 | 0.61 |
| 08 | 0.69 | 0.71 | 0.71 |

Every step's findings are the cold run's, and session = cold holds at every step. Step 05 is the one short of the
forms. Its kept run derives 39 of 121 strata again, every one past θ ("strata derived again, by reason", a new
decision line), and those cost what the cold run's do: `vopen` 291,842 rows and 0.42 s in both.

θ measured, kept instructions (G) per step:

| θ | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 |
|---|---|---|---|---|---|---|---|---|
| 0.25 (default) | 189 | 170 | 233 | 161 | 327 | 180 | 197 | 263 |
| 0.5 | 186 | 169 | 231 | 160 | 333 | 185 | 203 | 269 |
| off (`RERAIL_AGAIN=off`) | 196 | 215 | 350 | 199 | 506 | 227 | 201 | 396 |

Maintaining what θ derives again costs up to 1.5 times as much (05, 08), so the threshold is right. Between 0.25
and 0.5 the difference is within 2%, and θ stays 0.25 until the data layer's 9,010 files measure it too. Step 05's
gap is then the kept run's own overheads, not θ:

- the kept-facts writer's 2.9 s (a delta carries its predecessor's entries forward: 0353 row 12);
- `load` copying and hashing the inputs it reads from the frames (1.7 s);
- the input delta's owner reads (about 1.4 s, step 14).

## Step 16: a wave's kept puts are one block per space

Step 15's 2.9 s in the kept-facts writer was 112 jobs a run. Each input's job (`write_input_frames`) queued its own
kept puts, so each wrote a delta of the space and carried the previous delta's entries into it: carrying took 1.5 s
and finishing and syncing the blocks 1.0 s (new spans under "store writer: kept facts"). This is 0401 F2's
cumulative delta, written once per input rather than once per wave.

`queue_kept` now gathers the puts into the run's wave (`write_back::Gather`, as the tapes and the record are
gathered). When the wave is sealed, each space's puts are one block. A key put twice in the wave starts a second
block, so the puts apply in order (`a_waves_kept_puts_are_one_block_per_space`). Outside a run it is a job of its
own, as before.

O125 step 05, kept, on the M1 Max (load 13): 3 jobs and 0.115 s, against 112 jobs and 2.95 s. Kept instructions
at 01-05 are 181, 163, 217, 152 and 323 G, against step 15's 189, 170, 233, 161 and 327 G.

## Step 17: a changed input's old rows are its new rows of the unchanged documents

`Retained::load` read each changed input's old rows whole from the frames: every granule decoded into columns,
then every row hashed against its documents' digests (O125 step 05: 105 inputs; 1.0 s copying and 0.63 s
checking). Only the changed documents' old rows differ from the run's own new rows.

A changed input is now a `retain::OldInput`, a source over two parts:

- the runs of the new source's rows of the unchanged documents (`same_docs`, step 14), read in place;
- the changed documents' old rows, from their granules alone, checked against their digests and grouped by
  document. Their ranges are what `document_delta_except` reads the old side by.

It builds a column's lookup index where that column is first probed, as `Held` does. An input whose unchanged
documents hold other row counts than the last run's, or a semi-join's, is read whole, as before
(`shape::inputs_read_whole`). The R2 kept-path test asserts that count is 0 after each small edit.

O125 step 05, kept, on the M1 Max: 103 of 105 inputs are read this way. The load is 1.43 s against 1.65 s, since
the 14 changed documents (of 77) hold most of the rows. Kept instructions at 01-05 are 174, 157, 211, 145 and
321 G, against step 16's 181, 163, 217, 152 and 323 G. `RERAIL_LOAD_TRACE` prints each input's granules, rows
decoded and rows kept.

## Step 18: the forms' baseline checked, and why a cold run costs more

**The forms were not faster.** The forms' runs of 2026-11-17 (`tmp/o125/logs`) panicked at steps 05-08, qd and
kd alike, and wrote no findings: their 05-08 figures (qd 05: 171.5 G, "0.59") are the cost of a crashed run. Their
fractions were of step 00's cold run (289 G), not each step's own. Steps 01-04 are correct: their findings equal
this branch's cold runs, all 8,796-8,802. Those four are the only like-for-like comparison, in absolute
instructions:

| Step | Forms, DRed (qd) | Forms, counting (kd) | Maintain mode now |
|---|---|---|---|
| 01 | 184.3 G | 184.3 G | 173.9 G |
| 02 | 168.4 G | 168.4 G | 157.0 G |
| 03 | 214.7 G | 214.8 G | 211.5 G |
| 04 | 157.7 G | 157.7 G | 145.0 G |

The mode is 2-8% below the forms at each step, while the cold run under it costs 14% more than the forms' engine's
(below). Steps 05-08 have no forms figure: the forms did not finish them. Against cold, the mode's 05 is 0.87 (39
strata past θ, derived again at a derive's cost; with the threshold off it costs 1.5 times as much, step 15).

**Why cold rose.** The engine last built here on 2026-11-17 (12:29, before stages 2-6; its libraries and programs
from b5402793b, given by `RERAIL_SOURCE_ROOT`) and this branch's, on the M1 Max, cold O125 step 00, the same
10,475 findings: 306.7 G against 350.5 G (+14%). Profiled (which costs the old engine 54 G and this one 15 G), the
CPU that moved:

| Kind | Old | Now |
|---|---|---|
| an input's frames (job) | (in "other: cross") | 2.88 s |
| export to cross: base rows for the memo | 1.10 s | 2.23 s |
| export to cross: memo remember | 1.13 s | 1.69 s |
| local, its own | 0.06 s | 1.33 s |
| measuring sources | 0.40 s | 1.15 s |
| guests | 0.92 s | 1.57 s |
| other: cross | 3.33 s | 1.96 s |

Most of it is a cold run writing what later runs read: the memo's base rows and entries (now in segments), the
inputs' frames, and the kept facts written beside them (0401 step 6, never read: 0364's open items). The memo's
base rows now include `ast_depth`, R6's measure key (step 10), which is this work's share; it is not measured
apart. By the recorded runs, cold went from 307 G (2026-11-17) to 323 G at step 1 (stages 2-6, 2026-11-18) and to
about 361 G after the hub's merge (step 12): most of the rise came with the merge.

## Step 19: a new process's input deltas are views of ids, not copied rows

A changed input's delta was rows copied into new relations: each side's rows read into hash sets of `Vec<i64>`, their
differences sorted into a `Relation` (`delta_relation`), on the session path and the guests' rounds alike. Now
`retain::rows_delta` names the rows added and gone by their ids, as views over the new and old sources
(`InRel::picked`, as the session's patches are): each row is read once, with a 128-bit hash (Fx and `row_digest`),
and the two sides are matched sorted by it. `delta_relation` is gone. R2's kept test now asserts `rows_copied == 0`
on the new-process path too.

What a new process still decodes is counted apart: `shape::rows_decoded`, the kept rows and the changed inputs' old
rows decoded from the frames (0353's frames copy), 1,162,276 rows at O125 step 05. A test asserting it is 0 is
ignored with its reason until the reads bind the kept blocks (0401 step 7, on the branches being aligned).

O125 kept, M1 Max (loaded, 10-15): 01 171.7 G, 02 154.9, 03 209.9, 04 143.2, 05 320.9, 06 160.6, 07 173.6, 08 229.7,
findings equal to cold at every step. Step 05's matching went from 1.60 s to 0.96 s.

## Step 20: a session's base relations, patched by owner

At 2,410 files a session's 2-file edit took 54-59 s (M1 Max). Profiled, 21 s of it was the cross part's base
relations made again, whole, at every edit:

| Work, step 01 | CPU |
|---|---|
| the keeper's counted rows copied out (`columns_each`) | 1.4 s |
| those copied again into a block (`Held::new`) and bound | 6.1 s |
| each digested whole, to find the last run's source (`kept_source`) | 4.9 s |
| the semi-joins' rows measured | 2.1 s |
| each base input's index built again over every row (34) | 6.7 s |

A digest that changed (every tree relation, at any edit) found no last source, so every index was built again.
The kept route (a new process, decision 0117 step 2) had already removed this: a kept input is its kept rows with
its changed owners' rows laid over it (`retain::Layered`), read in place through the kept rows' index, its digest,
documents and delta from the changed owners alone (`patch_owners`). A session never took it.

Now it does. Each base relation that is not a semi-join's (whose rows follow the exports) is its last run's source
patched by the changed files' entries, old and new (`input_delta::changed`, the session memo), through the same
`patch_owners`. Its owners' ranges are found once, by one pass over the session's first source
(`retain::session_ranges`); after that the source is a `Layered` and its ranges are its own (`ranges_now`). A
second edit is laid over the same kept rows (`Layered::then`), not over the first edit's layer, so a session reads
one layer however long it runs; the rows of owners an earlier edit changed are carried into the new appended
block. The keeper's counted rows are no longer copied out for a patched relation. `RERAIL_SESSION_LAYERED=off`
rebuilds them as before.

Measured on the M1 Max:

| Files | Step | Before | Now |
|---|---|---|---|
| 2,410 | 01 | 54-59 s | 44.3 s |
| 2,410 | 02 | 65 s (M3) | 58.8 s |
| 2,410 | 03 | 55 s (M3) | 45.9 s |

At step 01 the base relations' part is 0.5 s (from 14.9 s), and the base inputs' indexes 0.6 s (from 6.7 s).
At step 03, after three patched edits, the session's findings equal a cold run's (4,343 G). O125's session equals
cold at every step, its CPU unchanged (its edits are of a few files over four documents'
worth of tree).

What a 2,410-file edit still costs, at step 03: `rr_agg37` and its demand stratum derived again where the mode is
priced above a derive (§4's second clause; 98,707 against 70,337 and 252,165 against 247,433, with deltas about
0.1% of the rows read), 16 s together with its pair statistics; a probe of a derived input's order, 4.1 s; the view
`kind`'s rows, 2.0 s; the semi-joins' rows measured, 1.9 s.
