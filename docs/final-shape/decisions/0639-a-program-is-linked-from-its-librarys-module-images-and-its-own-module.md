# 0639. A program is linked from its library's module images and its own module, as a library is

Status: draft (2026-11-23), for review. Applies decision 0246 §1–§2 (a module is lowered once, into its image; a
program is linked from images) to a single program's lowering, which still lowers every library file it includes
again. Builds on 0089 (libraries are modules), 0377 (lowering on versions) and 0264 (where images are kept).

## Intro

Before a run evaluates a program, it *lowers* it: the surface rules become core rules. A program that includes
library files (`#include "python/modules.rerail"`) is lowered together with every file it includes, and that walk
over the library's clauses is the same for every program that includes the same files. On a cold run of a small
Python program from the test store's template, the walk over the library's 539 rules was 38 ms of CPU (spark2,
2026-11-23), about an eighth of the run. Every test process pays it again, for every distinct program.

A pack's library is not lowered that way. Each library file (or group of files that include each other) is a
*module*, lowered once against the modules it includes and kept in the store as an *image* under a key of its
bytes and its scope. The pack's library is then *linked* from the images. A test (`tests_rr`,
`library_modules_link_to_the_lowering_of_them_together`) holds, for every pack, that the library linked from its
modules prints exactly as the library lowered as one.

This decision lowers a single program the same way: its library from module images, its own text as one more
module, the two linked, and the whole-program half of lowering run once over the result. The program it makes
prints identically to the one made today.

## How a program is lowered today

`rr::parse_version(src)` reads the text (`read_text`: its includes resolved through phase 0's graph, in include
order, the text's own statements apart) and calls `Text::lower_version(true)`:

1. `check_names` and `closed_program` on the text's own statements (decision 0263).
2. Every library file's statements, less their includes, then the text's own, in one list.
3. `lower_statements_version(list)`, which is `link_library_versions(vec![lower_module_in_written(list, &[])])`:
   - **The walk:** one module whose clauses are all of them, lowered against an empty scope
     (`lower_module_seeded_written`). This is the 38 ms.
   - **The link** of that one module (`lower::link_versions`), and what the lowerer knows of the whole
     (`whole_front`: kind sets, rewritten core relations, pending findings, three-valued and modal relations).
   - **The lowering's tail** (`lowering_tail_on`): a pipeline of whole-program passes over the linked version.
4. `contexts::passes` on the result.

A library goes through the same `link_library_versions`, with many modules instead of one
(`library_modules_with_files`: modules by the include graph, each read from its image or lowered against its scope
and kept).

## Why the two attempts failed

Two shortcuts were tried on 2026-11-23 and reverted. The code explains both failures.

**A single program through the pack path.** The combine's shared-library path (decision 0089) holds the library
once, as a part of its own: each program's own part, its requests' cones (`own_part`, `request_unit`), and a merge
that renames kind sets, hides and undoes answers on request across parts. That makes a different program by design,
not the same one faster. It also resolves includes through `read_at`, not `resolve_shared`, so the engine's phase
programs (`#include "core.rerail"` from `typing/programs`) did not resolve, and other texts failed "not among the
run's library files".

**The program's own part, then the library's relations copied in (`lowered_own` + `cone_on_version`).** The
lowering's tail is a pipeline of whole-program passes. `lowered_own` runs it over the text's own rules alone, and
the library's rules were copied in afterwards, so no pass of the tail saw them:

- `CoreDecls` declares the core relations the program's rules read. `node_order` and `tape`, read only by library
  rules, were never declared: 45 failures, "relation node_order is read by the rule for rr_agg24 and not declared".
- `KindSets` drops a set no rule reads, so a set read only by a copied rule was gone.
- `AggregateDemand`, `OrderNodes`, `SourcesOnRequest`, `WidenCyclicContexts`, `Findings` and `ThreeValued` each
  read the whole program. None of them composes over parts.
- `cone_of` leaves core relations out by design: a request copies the library's part, not the core's.

**What follows.** Only the walk is per module. The link and the tail must run once over the whole program. That is
`link_library_versions` given every module, the text's own included.

## The design

### 1. A text's own statements are one more module

For a modular text, `Text::lower_version(true)` becomes:

1. `check_names` and `closed_program`, as now.
2. **The library's modules:** `library_modules_with_files(paths, files, image_store())`, with the text's files in
   the order `read_text` lists them (includes first) and their statements less their includes. Each module is read
   from its image, or lowered and kept (decision 0246 §6), as for a pack.
3. **The text's module:** `lower_module_in_written(own, &scope)`, where `scope` is every module of step 2. The text
   reaches every file it lists, since the list is its includes' closure, and `core.rerail` where the list has it, as
   `library_modules_with_files` scopes a file.
4. **The link:** `link_library_versions(modules ++ [own module])`, and `contexts::passes`, as now.

A plain text (no includes) keeps `lower_statements_version`. `Text::lower_version(false)` (`parse_own_version`) and
`lower_library` (packs) are unchanged.

**Why the result is today's.** Today's list is the library's files in order with the text's statements last. Step 4
links that same list, cut into modules at the same boundaries `library_modules_with_files` cuts a library, and the
text is the last of them, reaching all before it. The pack test above holds that a list cut into modules and linked
prints as the list lowered as one, helper names included. So program identities, kept strata and stored images all
stay valid across the change.

**The switch.** `RERAIL_PROGRAM_LOWERING=whole` keeps today's path, to compare, as `RERAIL_LIBRARY_LOWERING=whole`
does for libraries, and is classified as that switch is (`every_switch_lowering_reads_is_classified`).

### 2. Modules are lowered and decoded once per process

`link_versions` takes its modules by value and changes only the first one's lowerer (the link's state). It will take
`&[Arc<Module>]`, cloning only the first module's `Lowerer`. A process-wide cache of modules by key
(`profile::shared`, like `lower_library`'s `Libraries`) lets a process that lowers many programs share them: one
`cargo test` process, a session, phase 1's programs. Today each would decode or lower the same module again.

### 3. A module's key does not depend on the order of its scope

A module's key hashes its files' digests and its scope modules' keys in the program's list order, and `seeds`
seeds them in that order. The same module can therefore be reached in two orders by two programs (A then B against B
then A, both including C), and then has two keys, so an image kept for one program is missed by the other.

- **The change:** the scope is taken in a canonical order, sorted by module key, for both the seeding and the key.
- **The precondition, tested first:** a module's lowering does not depend on its scope's order. `Lowerer::seed`
  adds inputs, declarations and violations in scope order, and `docof` is the last module's. `unseed` takes the
  seeded ones out again, but the test decides.
- **If the test fails,** the fields that carry the order are made order-free at the root, never worked around in
  the key.

### 4. The test store's template holds every module

`test_store::fill` lowers the whole library's modules into the template's store. That is every `.rerail` under the
rule library and the engine's phase programs (`fill` already lists them for phase 0), listed in include order by
phase 0's graph of one text including them all. They are drained with the rest. Each test's store is a copy of the
template, so with §3, every program a test lowers reads its library from images.

### 5. What a cold run then costs

| Step | Today | After |
|---|---|---|
| Lower the library's clauses (the walk) | 38 ms | none; each module's image is read and decoded |
| Lower the text's own clauses | yes | yes, in the scope's seeds |
| Link the modules (decision 0246 §2: rules copied, helpers renamed) | none | added |
| The lowering's tail | yes | yes, unchanged |
| `contexts::passes` | yes | yes, unchanged |

The change is kept only where the cold run's lowering costs less than today's. This is measured on the same cold run
(`tmp/cold2.sh` on spark2) before and after, by span.

## Tests, written before the code

- **T1.** Every pack program and every engine phase program: `parse_version` prints the same under the module path
  as under `RERAIL_PROGRAM_LOWERING=whole`. This extends the pack-library test to programs.
- **T2.** A program linked from its modules' images is the program linked from its modules lowered, as the library
  test holds for libraries.
- **T3.** Every library module lowered under its scope in two orders gives the same image bytes (§3's precondition).
- **T4.** A cold run of a Python program on a copy of the template reads every library module from its image: no
  module is lowered, by the `rr.module_image_read` and `rr.module_image_written` counters.
- **T5.** The corpus suite, and the rr tests, pass under the default and under `RERAIL_PROGRAM_LOWERING=whole`.

## Steps

1. T1 and §1. Every difference T1 shows is fixed at its root, in the module path, until the two print identically
   for every pack program and phase program.
2. §2: the borrowed link and the process's module cache.
3. T3, then §3.
4. §4 and T4.
5. §5's measurement, the suite, and the decision's status with the numbers.

## Risks

- **A text that adds to an `extensible` library relation** (decision 0263). Its module appends rules to a relation
  declared in its scope, as a library file may. T1 over the packs covers the programs that do.
- **A text in T's shape** (no `core.rerail`) has no core in its list, and so none in its scope, as for a library
  list without it.
- **Phase 0's own programs** resolve through `builtin_resolve` while phase 0 runs. Their list goes through the same
  step 2.
- **The template takes longer to fill** by the lowering of the whole library's modules, once per build. That is
  measured in step 4.
