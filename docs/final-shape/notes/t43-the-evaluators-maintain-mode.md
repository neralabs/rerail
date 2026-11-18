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
