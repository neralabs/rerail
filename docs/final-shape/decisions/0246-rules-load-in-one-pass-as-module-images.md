# 0246. Rules load in one pass: each module read once into its image, a pack linked from them

Status: draft (2026-11-15), for review. Makes decision 0225 §7 (a prepared program kept per module) specific for
loading, on decision 0173's program image as Typing-support builds it (the builder, views, overlays and derivations
of decisions 0215, 0241 and 0244). Builds on 0089 (a library is a module, held once), 0061 and 0162 (a rule file has
a tape, read in place), 0058 (hiding), 0131 (a store per project and build).

## Intro

Before a run evaluates its rules, it *loads* them. It reads the rule files, follows their includes, lowers the
surface syntax to core rules, combines the programs with their libraries, and prepares the combined program. For the
Python pack that is 317 programs and the library files they include: 1.3 MB of rule text.

Loading should cost a fraction of a second. It costs 2.7 s of CPU on one thread (36 G instructions), and 7.3 s on the
default pool. Measured on an M3 Max, release build, one Python file no earlier run saw, all 317 rules:

| store | CPU, one thread | instructions |
|---|---|---|
| fresh | 4.32 s | 57.2 G |
| the same rules already prepared in it | 1.59 s | 20.8 G |
| warm (the same file again) | 0.38 s | 5.4 G |

On the default pool the fresh run takes 9.5 s of CPU for the same instructions: 36% more cycles, spent on the pool's
small jobs.

No single pass is slow. About thirty steps each cost between 10 and 400 ms. The surveyed pipeline (below) shows
why. The same work is done many times, in a form that is rebuilt each time:

- **Each program text is parsed three times**: for its seed, for the batch's texts, and for combining. With the
  constants run, it is parsed four times. Its includes are resolved, and their digests checked, three times.
- **Every program lowers its libraries' declarations again.** `Text::lower` clones each included library's
  declarations into the program, lowers them, and `own_part` then deletes them. That is 318 lowerings of the same
  declarations: 317 programs, and the library once more.
- **Library statements are cloned per program** four times: to lower, to check relations, to list modules, and,
  after the run, to collect names for the run's record (0.27 s).
- **Congruence closure runs five times** over the whole program, `prune` three or four times, and
  `node_subject_tests` twice.
- **Whole-program passes ask whole-program questions** each time they run: strata, constants, locations, domains.
  `hiding::hide` is a fixpoint over all 7,611 rules (0.39 s), though 0089 §4 decided it per module.
- **The combined program is cloned** for the process cache, for the writer, and for each nested run. Every stored
  program is printed, parsed back, encoded as an image, decoded, and digested three times by `{:?}`.
- **Runs nest.** Phase 1 (`sources.rr`), each phase program, and the constants fold each start a full run of their
  own, and the constants run re-reads the 317 texts.

The forms are the cause. A text becomes a tape, then statements, then a `front::Program` tree for each program, then
one combined tree, then a lowered tree, then parts. Each step builds the next form whole, out of the last one. A
cold run builds every form for every program. Nothing is shared below the whole program, because nothing below it
has an identity.

## The shape

A rule file is a **module**. A module is read once, in one walk of its tape, into its own **module image**. A pack
is **linked** from module images, never rebuilt from their texts. The whole-program questions are **rules over the
image's relations**, evaluated as any rules are: for a cold run, the append of every module's rows.

```
rule file ──► tape (lean parser, 0162) ──► module image          one walk per module, per digest
                                             │  its own relations, its declarations, its rules (lowered),
                                             │  its includes (by module digest), its analysis rows
                                             ▼
pack (317 programs) ──► link: one base under the pack's ids      library modules once, each request's cone
                          │                                      once (0089 §3), each program's own rules
                          ▼
                       analyses = rules over the image           strata, hiding, congruence, constants,
                          │  (0225 §5), by delta: cold = append   locations, domains, the file-local split
                          ▼
                       prepare = derivations of the base         prune, simplify, demand, lower, push joins,
                          │  (0173 §9)                            factorize, locate: each an edit, by its size
                          ▼
                       parts = views + field overlays (0244)     local and cross parts, never copies
```

### 1. A module is read once, into its image

A module is a `.rr` file: a library file, a phase program, or a pack's program. Its identity is the digest of its
bytes, the grammar's version and the lowering's version.

The lean parser already reads a rule file into its tape (0162). The loader walks that tape once, and lowering writes
straight into the image builder: each surface form becomes the core rules it lowers to, emitted as image nodes. There
are no statements, and no `front::Program` tree.

Lowering has two halves today (`rr::lower::lower`), and they go to different places (decided 2026-11-15):

- **Per clause, into the builder.** Rewriting a clause (`match`, a finding, a setting, an aggregate, a text test)
  into core rules reads the clause and the declarations in scope. It writes image nodes through the builder's rule
  writer (`Builder::rule`, `RuleBuilder`), in the tape walk. It moves form by form; until a form has moved, its
  output from the tree path is the check, compared by digest.
- **Whole program, at link time.** The findings' rules, `declare_core`, projections, kind sets, boolean relations,
  `front::finish`, aggregate demand's `optimize`, removing repeated rules and literals, and widening cyclic contexts
  need the program with its includes. Today that is why every program copies its libraries' declarations in. They
  run once over the linked base (§2), as rules where their questions are rules (§3), and as one pass over the base
  where they are not yet.

A module image holds:

- **Its own relations**, by name. A relation the module only reads, declared by a module it includes, is a name
  in the image's string table, resolved when the module is linked (§2).
- **Its declarations, facts, settings and `.about` rows** in the image's regions (0173 §5), as the builder holds
  them today.
- **Its includes**, as module digests, not paths. The include graph is these rows.
- **Its own analysis rows** (§3): what each analysis derives from the module alone.

A module is read only if the store does not already hold its image (kept as below, "Aligned with django-bugs"). Each included file is read once per run, whatever includes it, and
once per store, whatever runs read it.

The run's record of the files it read takes their names from the module images' declaration region. There is no
second resolution.

### 2. A pack is linked from module images

Linking makes one **base**: the run's program, under one set of ids for the run. It holds:

- each library module once;
- each request's cone once per distinct request (0089 §3, keyed by the canonical text of the request's rules). A
  cone is made at link time (decided 2026-11-15): linking appends the library module's cone rules once more for
  each distinct request, under relation ids of its own. A copy is a renumbering, and linking numbers anyway, so a
  cone is never stored apart;
- each program's own rules, its relations numbered apart (0089's `p{i}_` names become distinct relation ids, with
  the program's index as the name's qualifier, printed only when a program is printed as text).

Linking appends module images under the run's ids. Each module's string table is interned once into the base's,
and its relation, rule and atom ids are offset. Nothing is lowered, cloned as a tree, or parsed again.

How a module's names become the run's ids is one interface, `LinkIds` (decided 2026-11-15): given a module image,
it gives the run's atom, relation and rule ids for the module's own. The first implementation re-interns each
module's string table into the base's and offsets the module's relation and rule ids. 0173 stage 4 (images keyed on
the store's atoms) would be a second implementation that only offsets; nothing outside `LinkIds` knows which one
runs.

This is a new builder entry, `Builder::link(modules)`, beside `derive` and `under`. Its cost is the size of what it
links: about 1 MB of image for the Python pack. A view's machinery (0244) does not need it: a linked base is a base.

### 3. The whole-program questions are rules over the image

The passes ask these questions of the whole program:

- the strata;
- which relations hide which documents (0058);
- which relations are equal (congruence);
- which relations are constants;
- which columns are locations;
- the domains;
- the file-local split.

Each is decided in 0225 §5 as rules over the image's relations, maintained by delta. Loading adds one thing: each
analysis's rows are kept **per module** where its rules let them be computed from the module alone. The module's
part of the strata, its hiding facts under each bound (0089 §4), and its own relations' congruence classes are
examples. Linking appends those rows; the rules then derive only what crosses modules.

A cold run is the append of every module's rows. A run whose modules are all stored derives only the cross-module
rows. An edit of one program re-derives what its module's delta reaches (0225 §2).

Congruence is one maintained relation, not five closures. `prune` and `needed` read one reachability relation.

### 4. Preparation is derivations of the linked base

`prune`, `simplify`, demand, `plan::lower`, pushing joins, factorizing exports, locating and splitting each read
the analyses (§3) and edit the program. Each is a derivation of the base (0173 §9): it costs the rules it changes,
and keeps its sources (`R::From`).

The parts are views of the prepared version with their fields as overlays (0244, `Version::part`). Nothing is
cloned.

### 5. Phases read the same image

Phase 0 derives the include graph over the rule files' tapes. That is the loader's input, and it stays a run of
rules (`bootstrap.rr`). Phase 1 (`sources.rr`), the phase programs, typing and the constants fold read the same
linked image in their own fact spaces. A phase never re-reads a text or starts a nested load.

### 6. The image is the stored form

A module image, a linked base and a prepared version are kept as images, each a source of the store under its
digest (0173 stage 1). The `.rr` text of a module is the source file the store already holds.

A stored program is not printed, parsed back, encoded and decoded, or digested by `{:?}`. Its digest is the
version's digest, kept as it is built (0173 §7).

### 7. Loading runs on one thread per module, not on the pool's small jobs

Reading modules is independent per module, so modules are read in parallel, one job each. Linking, the analyses'
append and the derivations run on the run's thread, or as one job per independent stratum of the analyses. No pass
fans out per rule (0178's pool loops go). The pool's 36% extra cycles were jobs smaller than their scheduling.

## What it should cost

These are estimates, to be checked by the stages' measurements:

- **Reading** the 1.3 MB of pack text plus the included library files: the lean parser and one builder walk, a few
  tens of ms.
- **Linking:** a copy of about 1 MB of image with its ids offset, a few ms.
- **Analyses, cold:** semi-naive evaluation over rows of the order of the image's nodes (about 10^5), tens of ms.
- **Derivations:** each the size of its change.

The target is **under 0.3 s of CPU, and under 2 G instructions,** for loading the Python pack cold on one thread,
against 2.7 s and 36 G today. A run on a store that already holds the modules should spend about the cost of
linking.

That target makes a store shared across projects unnecessary: a new project's first run loads its rules in the
time a shared store would save.

## Stages

Each stage ends with the intro's measurement: one file outside any earlier run, on a fresh store, a store that
already holds the rules, and warm; on one thread and on the pool; CPU and instructions. Findings are compared with the stage before.

1. **Merge Typing-support's 0244 work** (parts as views with their fields overlaid, `Version::part`), and the
   steps it builds on, into this branch, once Typing-support has committed it (decided 2026-11-15: wait for the
   commit, never merge work in progress). Every later stage edits versions, not trees.
2. **Module images.** Lowering writes into the builder from the tape (`rr::lower` → the builder), one module at a
   time. Library modules first: each is read once and stored under its digest. Programs keep their current path
   until stage 3.
3. **Linking.** `Builder::link` makes the run's base from library modules, request cones and programs' own
   modules. This replaces `combine_read` (parse #3, the per-program lowering of library declarations, the merge
   with renaming, `retype_helpers`, `same_relations`). The run's record reads names from the images.
4. **Analyses per module, as rules** (0225 §5), starting with the two that cost most cold: hiding (per module and
   bound, 0089 §4) and congruence (one maintained relation).
5. **Preparation as derivations** (0173 §9), and the parts as 0244's views.
6. **Phases on the linked image:** phase 1, the phase programs, typing and constants read it in their fact spaces.
   No nested run re-reads a text.
7. **The stored form is the image** (§6), and loading's pool loops go (§7).

## Stage 2, as built (2026-11-15)

Library modules are lowered one at a time and linked (`rr::library_modules`, `lower::lower_module_in`,
`lower::link`). The test `library_modules_link_to_the_lowering_of_them_together` checks every pack's library: the
linked program prints the same as the one lowering all its files together gives, and the same again when every
module is read back from its image. What building it decided:

- **A module is a file, or files that include each other.** `python/names.rr` and `python/imports.rr` include each
  other ("one mutually dependent unit"): they are one module, their statements in the run's order.
- **A module's scope is the modules it reaches, and only those.** Its clauses read their declarations, inputs,
  natives, kind sets, lattices, `must, may` relations, `.violation`s and what their relations read (`Lowerer::seed`).
  The result holds only its own (`Lowerer::unseed`), so its image is a function of its files and its scope's images.
  Lowering all files together let a file read what a file it does not include declares. No library relies on that.
- **What a module's text states is not what an include's lowering made.** A demand an include's lowering declared
  (`typing.rr`'s parse request) is not a printed demand of the module that includes it. Taken as one, it dropped
  `ruff-classes.rr`'s own requests, which would have missed findings.
- **Helpers are numbered per module and renumbered at link time.** `fresh`'s `rr_<stem><n>`, and the Python
  candidates serial, start at the module's first. Linking offsets them past the modules before it, which is the
  numbering lowering them all together gives. A helper's columns are in order of name, helpers by number
  (`helper_order`), so renumbering keeps their order. Linking also keeps each module's stated `.input`s before
  those its lowering added.
- **A module image is a program image** (one format, 0173). It holds the module's program, and its lowering's other
  parts as `.about` relations of their own, `module_*` (`rr/module_image.rs`). It is kept in the store under an alias
  of the image format, the engine build, its files' bytes and its scope's keys. A store that holds it reads it, and
  lowering does not run. `RERAIL_MODULE_IMAGES=off` skips the store; `RERAIL_LIBRARY_LOWERING=whole` lowers the
  files together, as before.

Not yet, and why:

- **Clauses are not written into the builder in the tape walk.** Each module's per-clause half still builds its
  `front::Program`, encoded once into the image. Writing image nodes clause by clause is the form-by-form move §1
  describes; the linked program's print is the check for each form.
- **Programs keep their current path** until stage 3, as planned. So does `lower_library`'s process map of linked
  libraries: stage 3's one link per run removes the reason for it.
- **A module image is pinned while a run holds it, not named in the run's record.** Collection may drop it, and the
  next run lowers that module again. Naming images in the record goes with stage 7 (the stored form).

Measured (M3 Max, one thread, the 317 converted Python rules, `logic` over one file of the corpus; process
instructions, CPU in seconds):

| Row | Before | Stage 2 |
|---|---|---|
| Fresh store | 60.49 G, 4.61 s | 60.12 G, 4.70 s |
| Store seeded by 10 files, then one file | 20.32 G, 1.54 s | 19.39 G, 1.49 s |
| Warm (the same run again) | 6.20 G, 0.43 s | 6.26 G, 0.44 s |
| One rule file edited on the seeded store | 42.66 G, 3.34 s | 43.59 G, 3.49 s |

With one rule file edited, the store holds every module's image but not the prepared program, which is the case
images are for. Reading them saves 0.2–0.3 G against `RERAIL_MODULE_IMAGES=off` (43.49 and 43.58 G against 43.78
and 43.90 G, two pairs), under 1%. On a fresh store, lowering one module at a time costs about 0.3 G more than
lowering the files together (`combine` 26.01 G against 25.70 G). So stage 2 is neutral on cost. The library's
lowering is a small part of loading. The edited row's 24 G `combine` is the 317 programs: each is lowered,
then combined with the library (`combine_read`). That is stage 3's work. Every output relation is the same before
and after, and with the library lowered together, on one file (144 rows) and on ten (1,044 rows).

## Stages 3, 4 and 7, as built so far (2026-11-15)

- **Every program is lowered in its libraries' scope** (`rr::Text::lower_in`, `lower::link_in`): its own clauses,
  as a module against the library modules its includes reach, and the whole-program half over them with the scope
  seeded; the scope's names are then taken out (core's and the kind sets stay, as `own_part` left them). No program
  lowers the library's declarations again: the Python pack made 110,927 relations a program lowered and `own_part`
  took out. A program that asks of the library (decision 0089 §3) gets its request's cone from the linked library
  (`with_cone`): made at link time, as §2 decided. The combined program before hiding: 5,788 rules (7,611).
- **Each program, and the linked library, is an image in the store**, under its text (or its modules) and its
  scope's module keys. A store that holds them reads them: nothing is lowered or linked again. An image is pinned
  with a lease (7 days) that each run reading it renews: `pin_held_sources` pins nothing outside a run's hold, so
  collection took every image at the end of its run (stage 2's were never read back). Naming them in the run's
  record is still to do.
- **Hiding is kept by delta across its rounds** (`hiding::Delta`): which rules reach a guest, what the columns gain
  (a least fixpoint started from the round before's), which are definite (a greatest fixpoint, made again over the
  cone of the change), and each round's analysis of only the rules whose inputs changed. Hiding the Python pack:
  0.07 s (0.40 s), release. The library's part of it is not yet kept with the library's image.
- **A stored program is its image** (`program_text::store`): encoded once, kept under its bytes' digest. Not printed,
  read back, encoded, decoded and digested by `{:?}` three times.

Measured (M3 Max, release, one thread, 317 rules, one file; instructions, CPU):

| Row | Before stage 2 | Stages 3 and 4 | With asking programs in scope, images as the stored form, scopes seeded once |
|---|---|---|---|
| Fresh store | 60.49 G, 4.61 s | 46.14 G, 3.55 s | 38.95 G, 2.99 s |
| Store seeded by 10 files, then one file | 20.32 G, 1.54 s | 19.42 G, 1.51 s | 19.23 G, 1.50 s |
| Warm | 6.20 G, 0.43 s | 6.23 G, 0.44 s | 6.22 G, 0.43 s |
| One rule file edited | 42.66 G, 3.34 s | 27.17 G, 2.15 s | 22.72 G, 1.80 s |

Every output relation is the same as before, on one file, on ten, and with a rule file edited. On the default pool
the fresh row takes 6.06 s CPU for 3.03 s wall (one thread: 3.06 s, 3.53 s): loading's pool loops are still to go. Loading cold is now
about 2.0 s: the programs 0.48 s (0.25 s for the 301 in scope), the library 0.11 s, the merge 0.18 s, preparation
0.67 s, the store's writer 0.43 s.

Open, found on the way: `typing::phase_tests::types_resolves_names_bases_members_and_annotations_by_symbol` fails
("a dotted annotation through `import pkg.m`"), at stage 2's commit too, so before this work.

## Aligned with django-bugs (decisions 0239, 0240)

- **Symbols by content.** A module image holds its names as strings, and a reader takes its own ids of them, as
  0239's output images and 0240 §4's memo entries do (and 0212's frames). So a module image means the same in every
  process and every store, and nothing maps ids at the store's edge. `LinkIds`'s first implementation is exactly
  that interning.
- **The stored form.** A module image is kept as 0240 §4 keeps a memo entry: one image, compressed where it is made
  (0240 §2), appended to the store's pack, found by a row keyed by the module's digest. The writer only writes
  (0240 §1): no run waits on it, and a queued image is read from memory until it is written.
- **One image format.** A module image is a program image (0173), built by the same builder. No third format.
- **Ids continue the stack's.** An overlay of a module image numbers its ids after its parent's, as 0173's overlays
  and 0239 stage 4's do.

## What this is not

- **Not a cache.** A module image is a module's facts, kept as any source's are (decision 0104). Linking and the
  analyses are evaluation over those facts.
- **Not a new evaluator.** The analyses are rules the engine runs (0225).
- **Not a change of findings.** Each stage is checked against the findings of the stage before.

## Open questions

- **Where lowering's checks go.** `tape_type::check`, `check_names` and `front::declared` run once per module. Those
  that need the module's includes run at link time, with lowering's whole-program half.
- **One layer of regions and strings.** The program image (0173) and django-bugs' output image (0239) have the same
  layout (header, fixed-width regions, string bytes and ends) in two codebases. Whether they share one reader and
  writer of regions and strings is for whichever change touches both first.

## The surveyed pipeline (2026-11-15, this branch)

A cold run's loading, in order:

| step | what | granularity |
|---|---|---|
| 0 | list program files; `program_texts` runs `bootstrap.rr`; each text parsed for its seed (parse #1) | per file |
| 1 | `Texts::of`: each text parsed again (parse #2); phase-0 graphs checked | ×317 |
| 2 | the components check re-reads each module's bytes | per module |
| 5–6 | `combine_keyed` → `read_text`: parse #3, includes resolved, every library file's statements checked | ×317 |
| 7–8 | `Text::modules`; `Text::lower` clones library declarations in, lowers, `aggregate_demand::optimize` | ×317 |
| 10–11 | `lower_library` (deep clone, one lowering); `own_part` deletes what step 8 copied | once; ×317 |
| 12–16 | merge with renaming; `retype_helpers`; `hiding::hide` (fixpoint); `scoping::guard` (congruence #1); `same_relations` (#2) | once |
| 17–18 | the combined program cloned for the process and for the writer; stored as text and image | once |
| 19–22 | phase 1, phase programs, the constants fold and typing: nested runs, the constants one re-reading the 317 texts | once each |
| 23–28 | `needed`, `fold_own` (#3), prune, simplify (#4), magic demand, matchers, `plan::lower`, push joins, factorize, locate (#5), domains, the split (`assemble` clones twice) | once |
| 29–31 | prepared program stored (print, parse back, encode, decode, three `{:?}` digests); DFAs compiled; parts become versions (`Placed::of_program`) | once |
| 32 | after the run: `reuse::record` resolves every program's includes again for names | ×317 |
