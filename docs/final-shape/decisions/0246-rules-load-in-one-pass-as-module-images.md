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

A module image holds:

- **Its own relations**, by name. A relation the module only reads, declared by a module it includes, is a name
  in the image's string table, resolved when the module is linked (§2).
- **Its declarations, facts, settings and `.about` rows** in the image's regions (0173 §5), as the builder holds
  them today.
- **Its includes**, as module digests, not paths. The include graph is these rows.
- **Its own analysis rows** (§3): what each analysis derives from the module alone.

A module is read only if the store does not already hold its image (0173 stage 1: the image is a source of the
store, under an alias of the module's digest). Each included file is read once per run, whatever includes it, and
once per store, whatever runs read it.

The run's record of the files it read takes their names from the module images' declaration region. There is no
second resolution.

### 2. A pack is linked from module images

Linking makes one **base**: the run's program, under one set of ids for the run. It holds:

- each library module once;
- each request's cone once per distinct request (0089 §3, keyed by the canonical text of the request's rules);
- each program's own rules, its relations numbered apart (0089's `p{i}_` names become distinct relation ids, with
  the program's index as the name's qualifier, printed only when a program is printed as text).

Linking appends module images under the run's ids. Each module's string table is interned once into the base's,
and its relation, rule and atom ids are offset. Nothing is lowered, cloned as a tree, or parsed again.

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
   steps it builds on, into this branch. Every later stage edits versions, not trees.
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

## What this is not

- **Not a cache.** A module image is a module's facts, kept as any source's are (decision 0104). Linking and the
  analyses are evaluation over those facts.
- **Not a new evaluator.** The analyses are rules the engine runs (0225).
- **Not a change of findings.** Each stage is checked against the findings of the stage before.

## Open questions

- **The ids a module image is linked under.** Linking re-interns each module's names into the run's string table
  (§2), which costs the size of the names. The alternative is 0173 stage 4: images keyed on the store's atoms, so
  linking only offsets relation and rule ids. Stage 4 is not built.
- **Where lowering's whole-module checks go.** `tape_type::check`, `check_names` and `front::declared` run once per
  module. Some of them need the module's includes, and so run at link time.
- **Request cones.** Whether a cone is a module of its own (stored under its request's digest), or a derivation of
  its library's module made at link time.

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
