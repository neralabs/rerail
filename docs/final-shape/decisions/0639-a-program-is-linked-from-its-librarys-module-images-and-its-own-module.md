# 0639. A program is linked from module images that phase 0 plans and keeps as facts

Status: draft (2026-11-23, revised the same day at the user's direction: keep as much as possible as facts, and do as
much of the work as possible in `.rerail` rules). Applies decision 0246 §1–§3 (a module is lowered once into its
image; a program is linked from images; whole-program questions are rules) to a single program's lowering, under
0104 (everything kept is facts, in 0103's two-rule form). Builds on 0089 (libraries are modules), 0377 (lowering on
versions) and 0264 (where images are kept).

## Intro

Before a run evaluates a program, it *lowers* it: the surface rules become core rules. A program that includes
library files (`#include "python/modules.rerail"`) is lowered together with every file it includes. That walk over
the library's clauses is the same for every program that includes the same files. On a cold run of a small Python
program from the test store's template, it was 38 ms of CPU for the library's 539 rules (spark2, 2026-11-23), about
an eighth of the run. Every test process pays it again, for every distinct program.

A pack's library is not lowered that way. Each library file, or group of files that include each other, is a
*module*: lowered once against the modules it includes, and kept as an *image*. The pack's library is *linked* from
the images. But the plan of the modules (the include order, which files form a module, each module's scope and its
key) is Rust code in `library_modules_with_files`, and the images are opaque blobs under Rust-made keys.

This decision does three things:
- **A single program is lowered like a library:** its library from module images, its own text as one more module,
  and the whole-program half of lowering run once over the linked result.
- **Phase 0 plans the modules in rules.** Phase 0 already derives the include graph in `bootstrap.rerail`. It now
  also derives the module plan.
- **What is kept is facts:** each module's key, its plan, its lowering's parts as rows, and a reference to its image,
  in 0103's two-rule form. Rust is left with the steps that are inherently code: walking a module's clauses,
  copying image bytes in the link, and the lowering's tail passes not yet expressed as rules.

## How a program is lowered today

`rr::parse_version(src)` reads the text (`read_text`: includes resolved through phase 0's graph, the text's own
statements apart) and calls `Text::lower_version(true)`:

1. `check_names` and `closed_program` on the text's own statements (decision 0263).
2. Every library file's statements, less their includes, then the text's own, in one list.
3. `lower_statements_version(list)`, which is `link_library_versions(vec![lower_module_in_written(list, &[])])`:
   - **The walk** over every clause (the 38 ms).
   - **The link** of the one module (`lower::link_versions`, `whole_front`).
   - **The lowering's tail** (`lowering_tail_on`): a pipeline of whole-program passes.
4. `contexts::passes`.

A library goes through the same `link_library_versions` with many modules. A test
(`library_modules_link_to_the_lowering_of_them_together`) holds, for every pack, that a list of files cut into
modules and linked prints exactly as the list lowered as one, helper names included.

## Why the two shortcuts failed

**A single program through the pack path.** The combine's shared-library path (0089) holds the library as a part of
its own, with each program's own part, request cones, and a merge that renames kind sets, hides and undoes answers
on request across parts. That is a different program by design. It also resolves includes through `read_at`, which
did not resolve the engine's phase programs (`#include "core.rerail": no such file`).

**The program's own part, with the library's relations copied in (`lowered_own` + `cone_on_version`).** The tail ran
over the text's own rules alone, and the library's rules were copied in after it, so no pass saw them:
- `CoreDecls` declares the core relations the program's rules read. `node_order` and `tape`, read only by library
  rules, were never declared: the 45 failures.
- `KindSets` drops sets no program rule reads.
- `AggregateDemand`, `OrderNodes`, `SourcesOnRequest`, `WidenCyclicContexts`, `Findings` and `ThreeValued` each read
  the whole program.

**What follows.** Only the walk is per module. The link and the tail run once over the whole program.

## What is facts, what is rules, what is Rust

| Work | Today | Here |
|---|---|---|
| Include edges | rules (phase 0) | rules (phase 0), unchanged |
| Include order, reach, module groups, scopes | Rust (`phase0::Graph::order`, `library_modules_with_files`) | **rules** in phase 0 (§1) |
| Module key | Rust (`module_key`) | **rules** (§1), with `@sha256` and the ordered `join` aggregate |
| Whether a module must be lowered | Rust (a store lookup) | **rules**: a request where no result is kept (§2) |
| Walking a module's clauses | Rust | Rust, the request's native step (§2) |
| A module's parts (violations, deps, kind sets, helpers, ...) | inside an opaque image | **facts**, rows per module (§2) |
| A module's rules | opaque image | one block per module, its digest in a fact (§2) |
| Link order, helper merges, field merge order | Rust (`link_versions`) | **rules** decide (§4); Rust copies bytes by them |
| `whole_front` (sets, rewritten, findings, three, modal) | Rust, from the lowerer | **rules**: unions over part rows (§4) |
| Tail: which core relations are read, which kind sets are read | Rust passes | **rules** answer, the passes apply (§5) |
| The other tail passes | Rust | Rust, unchanged (§5) |

Efficiency bounds what becomes a fact. Rows are per file, module, relation or helper: hundreds to thousands for the
Python library, never per rule node. A module's rules stay one block, read in place (`Version::open_shared`), as
decision 0246 §6 and the store's block rules require. No process-wide cache is added. A module is read from its kept
facts and block.

## The design

### 1. Phase 0 derives the module plan

`bootstrap.rerail` gains the plan, over its existing outputs (`p0_include`), the loaded documents' digests and two
builtin facts the engine states with the others: `env("rerail.build", id)` (the engine's build identity) and
`env("rerail.lowering", s)` (the lowering switches' values that `rr::switched` puts in keys today). The relations, as
sketches (column types as in `bootstrap.rerail`):

```
// Each file reached from a text or module, transitively.
.decl p0_reaches(from: symbol, path: symbol)
p0_reaches(f, p) :- p0_include(f, _, _, p).
p0_reaches(f, p) :- p0_reaches(f, q), p0_include(q, _, _, p).

// A module: files that reach each other are one. Its name is its least member's digest.
.decl p0_same_module(a: symbol, b: symbol)
p0_same_module(a, a) :- p0_digest(a, _).
p0_same_module(a, b) :- p0_reaches(a, b), p0_reaches(b, a).
.decl p0_module_of(path: symbol, m: symbol)
p0_module_of(p, m) :- p0_digest(p, _), m = min d : { p0_same_module(p, q), p0_digest(q, d) }.

// A module's scope: the modules its files reach, itself apart (core.rerail is reached as an include).
.decl p0_scope(m: symbol, s: symbol)
p0_scope(m, s) :- p0_module_of(p, m), p0_reaches(p, q), p0_module_of(q, s), s != m.

// Its key, from what decides its lowering, with no recursion through the aggregate: its members' digests, the
// digests of every file it reaches, and the include edges among them, each list ordered by digest.
.decl p0_module_key(m: symbol, key: symbol)

// The order modules link in: a module after every module in its scope. A scope's reach is a strict subset, so
// ordering by the count of files reached, then by key, is a topological order that depends on no program.
.decl p0_module_rank(m: symbol, reached: number, key: symbol)

// A text's modules, in that order (what `read_text` lists today in DFS order).
.decl p0_text_module(t: symbol, m: symbol)
.output p0_module_key, p0_module_rank, p0_text_module, p0_scope
```

The members of a module are ordered by digest too. So a module's key, its statements and its place in a link
depend on its own files and what they reach, never on which program reached it or in what order. That makes
§3-of-the-first-draft's scope ordering unnecessary: the plan is canonical by construction.

**What this changes.** The link order is canonical where it was each text's DFS order, so a program's printed rule
order and helper numbers change once. Relation identities change with them, as with any change of the engine's
build, which the template, stores and images are keyed by. The tests (below) compare the module path against the
whole lowering of the same canonical list, and the suite's outputs against today's.

### 2. A module's lowering is a request, its result facts (0104's two rules)

- **The key fact:** `p0_module_key(m, key)`, from §1.
- **The result:** `p0_module_lowered(key, image)` (`image` a digest naming the module's block) and its part rows,
  answered by the engine on request, in phase 0's fixpoint, as `load` is.

```
.decl p0_lower_ask(key: symbol)
p0_lower_ask(k) :- p0_module_key(m, k), !p0_module_lowered(k, _),
    count : { p0_scope(m, s) } = count : { p0_scope(m, s), p0_module_key(s, sk), p0_module_lowered(sk, _) }.
```

A module is asked for only once every module in its scope is lowered (the count test), so requests run in rounds
in dependency order. A kept result is no delta: a warm run, or a test store copied from the template, asks for
nothing. The engine's native step for a request:
1. Reads the module's statements off its files' tapes, which phase 0 loaded.
2. Seeds the scope from the scope modules' blocks and part rows.
3. Walks the clauses (`lower_module_seeded_written`).
4. Writes the image block, and answers the request with the image's digest and the part rows.

**The part rows** are what `lower::ModuleParts` holds today, already rows of names and numbers:

| Relation | Rows |
|---|---|
| `p0_module_violation` | `.violation`s: name, arity, code, node, places, message, tag |
| `p0_module_violation_column` | each violation's columns |
| `p0_module_dep` | `(rel, read)`: what each derived relation reads |
| `p0_module_written` | each relation's column types as written |
| `p0_module_helper` | helpers made: name, stem, number, rule count, template key (`once_key`) |
| `p0_module_three`, `p0_module_modal`, `p0_module_set`, `p0_module_lattice`, `p0_module_place`, `p0_module_demand` | the remaining parts |
| `p0_module_docof` | whether `docof` is over locations |

Each is keyed by the module's key. `Module::of_parts` reads these rows and the block, where it decodes an opaque
image today.

### 3. A program's own text is a module too

The text (`p0_text(t)`) is one more module: its key is the text's digest with the keys of its scope (every module
it reaches), and it is lowered on request as in §2. It is the last in its link. A program lowered once is not
walked again in any process whose store holds it: a test process's programs after the first, and every test whose
program the template ran.

### 4. The link: rules decide, Rust copies

`link_versions` makes three decisions today that are questions over the part rows. They become rules:

| Question | Rule |
|---|---|
| Link order | `p0_text_module` by rank (§1) |
| Helper merges (`once_key`) | `p0_link_merge(t, helper, into)`: a helper whose template key an earlier module in the link made is read as that module's |
| Helper renames (number clashes) | `p0_link_rename(t, m, helper, name)`: helpers numbered in link order |

`whole_front`'s answers are unions over the part rows in link order: kind sets, rewritten core relations, pending
findings (violations, numbered on in link order), three-valued and modal relations. These are rules too:
`p0_link_set`, `p0_link_finding`, and so on.

Rust's link then only copies: each module's block appended under the run's ids, with the renames and merges read
from the facts (`relinked`, which takes the maps today), and the fields appended in link order (`append_fields`).
`link_versions` takes the modules by reference and clones no module's lowerer.

### 5. The lowering's tail

The tail's passes stay Rust in this decision, run once over the linked program as today. Two of them ask set
questions over the whole program, which the per-module rows answer by rules:

- **`CoreDecls`:** which core relations the program's rules read. `p0_link_reads_core(t, rel)` is the union of the
  modules' `p0_module_dep` reads that are core relations, plus the core relations the core's own rules read.
- **`KindSets`:** which declared kind sets some rule reads as an atom. `p0_link_set_read(t, set)` is computed the
  same way, from the module rows that name the sets their rules read.

Each pass takes the rules' answer. In debug builds it checks the answer against its own computation, as 0333's
stored analyses are checked. `AggregateDemand`, `OrderNodes`, `SourcesOnRequest`, `WidenCyclicContexts`, `Findings`
and `ThreeValued` rewrite rules, and stay passes. Moving their questions to rules is 0246 §3 and 0225 §5's work,
not this decision's.

### 6. One mechanism for packs and single programs

`lower_library` reads the same facts. Its Rust graph code goes: `library_modules_with_files`'s reach, groups, order
and `module_key`, and `phase0::Graph::order` where only the lowering used it. A pack's library is the link of its
programs' library modules, by rank. The pack-library test holds the pack path to the same output.

### 7. The template holds every module

`test_store::fill` gives phase 0 one text that includes every library file and every engine phase program, and asks
for every module. The facts and blocks land in the template's store. With §1's canonical keys, every test's
programs find their modules lowered.

### 8. Phase 0's own programs

`bootstrap.rerail` is lowered before phase 0 runs. It, and anything lowered while phase 0 is active
(`builtin_resolve`), keep the whole lowering (`lower_statements_version`). The template holds bootstrap's plan as
today.

## Tests, written before the code

- **T1.** For every pack program and every engine phase program, the module path prints the same as the whole
  lowering of the same canonical list (§1), helper names included.
- **T2.** The suite's outputs are today's. The corpus suite and the rr tests pass under the default and under
  `RERAIL_PROGRAM_LOWERING=whole` (today's path, kept to compare).
- **T3.** The plan is canonical: two texts that include the same files in different orders have the same
  `p0_module_key` and `p0_module_rank` rows for every module.
- **T4.** A module's part rows read back with its block give the module that was lowered (`Module::of_parts` against
  the lowered `Module`: the same parts, the same version digest).
- **T5.** A cold run of a Python program on a copy of the template asks for no module (`p0_lower_ask` is empty), and
  a second run of the same text asks for none either.
- **T6.** The link facts (`p0_link_merge`, `p0_link_rename`) and the tail answers (`p0_link_reads_core`,
  `p0_link_set_read`) equal what `link_versions`, `CoreDecls` and `KindSets` compute.

## Steps

1. **§1:** the plan in `bootstrap.rerail`, with T3, beside the Rust plan, checked equal on every pack.
2. **§2 and §3:** the request, its native step, the part rows and blocks, with T4 and T5.
3. **§4:** the link's rules and the copying link, with T1 and T6. Then single programs switch to the module path
   (T2).
4. **§5:** the two tail answers, with T6.
5. **§6:** `lower_library` onto the facts, and the Rust plan removed. **§7:** the template.
6. Measure the cold run by span (`tmp/cold2.sh`, spark2) before and after. Record it here.

## Risks

- **Recursion through an aggregate** is avoided by construction: a key reads the closure's digests, not the scope's
  keys, and the ask's count test reads only lowered results of other modules.
- **The count test** waits on a scope module that fails to lower. The failure is answered as a result row with the
  error (as a missing include is `p0_missing`), and the text's lowering reports it.
- **A text that adds to an `extensible` relation** (0263) appends rules to a relation declared in its scope, as a
  library file may. T1 over the packs covers it.
- **Phase 0's run grows** by the plan's rules and the requests. Its rows are per file and module, and a kept plan is
  no delta. The cold template fill takes the whole library's lowering once per build, measured in step 5.
