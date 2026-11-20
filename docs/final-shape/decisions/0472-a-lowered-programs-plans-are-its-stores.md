# 0472. A lowered program's plans are its store's: they do not move across stores

Status: accepted (2026-11-19), Q291's decision. Made in the work queue's order, by the agent doing the work; the
user may overrule it.

## Intro

A rule is lowered against a store: each name it holds becomes the store's atom for it (decision 0270). So everything
hashed over a lowered rule depends on the store's numbering: rule hashes, the file-local helpers' names
(`typing::file_local::helper_name`), a version's identity (`manager::identity`), and a line's key (`line_key`). The
same rules lowered on two new stores get two identities (Q291: `#n2708` on one store, `#n13220` on the other). Within
one store all of these stay the same, because the store keeps its atoms.

Q291 asked whether plans should move across stores. If they should, every one of those hashes would have to be over
the names' text and not their atoms.

## Decision

They do not. What a run keeps after lowering stays the store's, keyed by the store's atoms:
- the kept plans (decisions 0038 and 0116);
- the prepared plans (Q183);
- the stored pipeline runs;
- the memo.

Two things cross stores today, and both are keyed by something with no atoms in it:
- the rule images, keyed by the engine's build, the image format and the source bytes (decision 0264);
- the machine's learned cost model (decision 0442's `Machine`).

## Why

1. **They live in the store.** The kept plans and prepared plans are rows in the project's content store
   (`choices::read_plans(store, …)`). A new store holds none, so a store-independent identity would find nothing to
   read. Moving plans across stores needs a place they are shared in, as well as the new keys.
2. **Such a place carries one project's data to another.** A kept plan holds a line's work and rows per node on the
   corpus that made it. Under decision 0442, data shared between callers comes only from an explicit `Shared` object
   the caller passes, so that security boundaries hold. No caller asks for plans from another project.
3. **There is little to win.** Planning is 1-3% of a cold run's CPU (0361 step 5, corpus0 x B-now: 0.3 s of search
   and about 2.7 s of statistics out of 94 s). A new store's prepared plans are made from the facts at its first
   run, with no store read (Q183's "made" count).
4. **The other way costs every key.** Rule hashes, helper names, identities and line keys would all be re-based on
   text: a name lookup at each atom of every hash, `line_key` included, which is computed at every plan.

## What a new store does

Its first run makes its versions' prepared plans and searches the rest. Its later runs read what the first one kept.
A caller that wants to seed a new store, such as a CI job from a checked-in plan file, would export and import a
store's plans explicitly, keyed by the program before lowering. That is not built.

## Open

- If a seeded store is ever wanted: the export and import above, through an explicit shared object (0442), keyed by
  the rules' text and their delta.
