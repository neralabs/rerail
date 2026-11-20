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

