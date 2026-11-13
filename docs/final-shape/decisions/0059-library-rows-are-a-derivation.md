# 0059: A library's cross-file rows are a derivation, replayed into the workspace run

Status: withdrawn (2026-11-07); the code is removed. Replaced by 0065.

**Why withdrawn.** It made the library a second, separate run (a "stage"), and copied its rows into every round of
the workspace run. Both are wrong in shape. Nothing about a library is special: its files are documents like any
other, and their facts are stored and reused like any file's because their contents are unchanged (0065). The measurements below are kept as evidence: replay into
bounded rounds cost more than it saved.

## Intro

A typing run over a workspace now loads library documents: typeshed's stubs for the standard library,
and later the stubs and sources of installed packages (t21). On an 84-file workspace, 179 stubs are
loaded, and most of the run is spent on them:

```text
// calls.rr at k = 1, debug build, load 60-130 (CPU s)
                         local   cross
workspace + its stubs     8.7    12.6
the stubs alone           6.9    10.3     (a one-file workspace importing the same modules)
```

The local part of a stub is already made once: the per-file memo (0052) keys a document's rows by its
content, path and plan, and a stub's path is its canonical path, the same in every workspace.

The cross-file part is not. Every run derives the stubs' classes, bases, inherited attributes and
re-exports again, and each bounded round (0035, 0047b) derives them again from the start. These rows
do not depend on the workspace: a library never imports it. They are the derivation the store's
graph (03 §14, `content_store::derivation`) exists for, at one more granularity: one per set of
library documents closed under their imports, not one per file (t21 §2.5).

The spike's indexing work (prior-work design index §20, §4) is the same lesson at file granularity.
Its statistics had to be per file and persisted (D S124: no statistic may need the whole corpus;
DH L417: state held only in memory is "dishonest"). A library's rows are kept in the store with
their statistics, not in process memory.

## Decision

1. **The library stage.** After the requests converge (`world::discover`), a run with loaded
   documents first runs the program over the loaded documents alone (`world: "library"`):
   - no workspace document, so no row about one, and no finding is written;
   - the program's own demand is opened for the stage. A program reads `stage("library")` (engine
     fact) to need every attribute and name a library binds, since a workspace may ask for any of
     them. The engine's demand (0048) is not applied: a negated relation is computed for every key;
   - its rounds run to the fixpoint, or to a bound of their own (an open question of t21 §7);
   - the rows of every relation the cross-file part derives are the stage's output.

2. **The derivation.** The output is stored as a blob, and recorded with
   `ContentStore::record_derivations`:
   - output name: `library-rows/<digest of the inputs>`;
   - producer: the cross-file part as lowered (its atoms included) and the engine build;
   - inputs: each loaded document's store path bound to its content digest, and the settings
     (`env` facts) the program reads.

   A later run with the same inputs reads the blob and skips the stage. A changed stub rebinds its
   path, so the derivation is invalid and the stage runs again, as for any changed file.

3. **Replay.** The workspace run binds each relation's library rows as an input
   `<rel>__library` and adds `rel(...) :- rel__library(...)` to its cross-file part. Its cross-file
   rules derive no row whose first node column lies in a library document (a guard on the head's
   node, `@doc(n)` not a library document): those rows are the stage's, complete. Rows keyed by a
   symbol (a qualified name) are derived as before.

   A row about a library node that only the workspace's demand would make is the stage's already,
   because the stage's demand was open.

4. **What stays the same.** The local part of every document, library or workspace, is the
   per-file memo's. The workspace's rounds and their bound are unchanged. Rows about library
   documents are still not written as output (t21 E2).

## Order of work

1. The stage, its rows, and replay with the head guard, behind the setting `library.rows`
   (`off` by default since Measured; `on` replays), to compare the two.
2. `calls.rr`: `stage("library")` opens `need_attr`, `need_name` and the star names (`wantx`).
3. Measured on the 84-file corpus: the Jedi score unchanged, and a warm run costing at most 10%
   more than the run without typeshed (t21 step 2's goal).

## Measured (2026-11-07)

A one-file workspace importing the same modules as the 84-file corpus (149 library documents), `calls.rr` at k = 1,
debug build, load 15-100 (CPU s of the cross-file part):

```text
// library.rows   rows replayed   cross
off               -                9.2   the workspace derives what it asks of the library, under its demand
on (warm)         1,062,869       12.5   the stage's rows, read from the store, then the same rounds
```

Replay does not pay, for two reasons the design missed:

- **The stage's open demand.** The workspace asks little of the library (its `need_attr` rows are a few thousand),
  so the rows it derives about library nodes are few. The stage, open for any workspace, derives every class's
  every attribute: a million rows, which cost more to load and index than the few to derive.
- **Rows are loaded per round.** The rounds (0035, 0056) run the cone again each round, and each round loads the
  replayed rows into the engine and indexes them again.

Two changes the first measurements forced stay, and serve any later form of this decision:

- Replayed rows are a component's *closed* rows (`setfix`'s `rel ... closed <table>`): rows at the lowest ids, of
  an earlier wave, never a delta. Without it every rule joined the whole library again in the first wave and
  discarded the rows at the head guard (53 s, not 12).
- Only relations in or after a recursive component are kept (`library::kept`); a one-pass relation (`kind`, a walk)
  costs what reading it back costs.

A form that could pay reads the stored rows in place, by index, as the workspace's demand asks (no load), and
once per run, not per round. Until then the setting is `library.rows=on` to measure, off otherwise.

Measured with it: what the rounds read and no round changes runs once before them (`cross_rounds::pre_split`):
69 of the cone's relations, 1.9 s once instead of each round.

## Not decided here

- Splitting the stage per import-closed component (a changed stub would then re-run only its own
  component). One stage per run's loaded set first.
- Rows in a library document that the workspace changes. None exist while a library cannot import
  the workspace, and values do not flow from a call's arguments into a library's parameters.
