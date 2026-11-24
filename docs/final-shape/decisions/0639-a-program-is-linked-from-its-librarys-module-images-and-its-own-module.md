# 0639. A program is linked from module images that phase 0 plans and keeps as facts

Status: draft (2026-11-23). Revised the same day at the user's direction (keep as much as possible as facts, and do
as much of the work as possible in `.rerail` rules), then spiked: every design decision below was checked by a run
or by the code it depends on (§ Spike). Applies decision 0246 §1–§2 (a module is lowered once into its image; a
program is linked from images) to a single program's lowering, under 0104 (everything kept is facts, in 0103's
two-rule form). Builds on 0089, 0377 and 0264.

## Intro

Before a run evaluates a program, it *lowers* it: the surface rules become core rules. A program that includes
library files is lowered together with every file it includes. That walk over the library's clauses is the same for
every program that includes the same files, and every test process pays it again for every program.

A pack's library is not lowered that way. Each library file, or group of files that include each other, is a
*module*: lowered once against the modules it includes, and kept as an *image*. The library is then *linked* from
the images. But the plan of the modules (which files form each, its scope, its key, the order) is Rust code in
`library_modules_with_files`, and a single program does not use the images at all.

This decision:
- lowers a single program as a library is lowered: its library from module images, its own text as one more module,
  and the whole-program half of lowering run once over the linked result;
- derives the module plan in phase 0's rules, where the include graph is already derived;
- makes each module's lowering a request in phase 0, answered by the engine, its result kept in 0103's two-rule
  form.

- names each helper a module makes by its content (or, for a helper no lowering merges, by its module), so that
  linking modules is a union with nothing left to decide, written as rules, and run by the engine as a view over the
  modules' images.

What stays Rust: the walk over a module's clauses (the request's native step) and the lowering's tail, whose
whole-program passes are 0246 §3's work.

## How a program is lowered today

`rr::parse_version(src)` reads the text (`read_text`: includes resolved through phase 0's graph, the text's own
statements apart) and calls `Text::lower_version(true)`:

1. `check_names` and `closed_program` on the text's own statements (decision 0263).
2. Every library file's statements, less their includes, then the text's own, in one list.
3. `lower_statements_version(list)`: one module of the whole list, walked against an empty scope, linked alone
   (`link_versions`, `whole_front`), and the lowering's tail (`lowering_tail_on`) run over it.
4. `contexts::passes`.

## Why the two shortcuts failed

**A single program through the pack path.** The combine's shared-library path (0089) holds the library as a part of
its own, with each program's own part, request cones, and a merge across parts. That is a different program by
design. It also resolves includes through `read_at`, which does not resolve the engine's phase programs.

**The program's own part, with the library's relations copied in (`lowered_own` + `cone_on_version`).** The tail ran
over the text's own rules alone, so no pass saw the library's: `CoreDecls` left `node_order` and `tape` undeclared
(the 45 failures), `KindSets` dropped sets only library rules read, and `AggregateDemand`, `OrderNodes`,
`SourcesOnRequest`, `WidenCyclicContexts`, `Findings` and `ThreeValued` each read the whole program.

**What follows.** Only the walk is per module. The link and the tail run once over the whole program.

## The design

### 1. Phase 0 derives the module plan

`bootstrap.rerail` gains these rules over its include edges and each module's digest. They are as the spike ran them
(E5), with the edges taken from `p0_include` and the digests from the `file_digest` answer:

```
.decl p0_edge(a: symbol, b: symbol)
p0_edge(a, b) :- p0_include(a, _, _, b), p0_digest(a, _), p0_digest(b, _).
.decl p0_reaches(a: symbol, b: symbol)
p0_reaches(a, b) :- p0_edge(a, b).
p0_reaches(a, c) :- p0_reaches(a, b), p0_edge(b, c).
// A module: a file with the files that reach it and that it reaches. Its name is its members' digests, in order.
.decl p0_member(p: symbol, q: symbol)
p0_member(p, p) :- p0_digest(p, _).
p0_member(p, q) :- p0_reaches(p, q), p0_reaches(q, p).
.decl p0_module_of(p: symbol, m: symbol)
p0_module_of(p, m) :- p0_digest(p, _), m = join d by d sep "+" : { p0_member(p, q), p0_digest(q, d) }.
// Its closure: its own files and every file they reach.
.decl p0_closure(m: symbol, q: symbol)
p0_closure(m, q) :- p0_module_of(p, m), p0_member(p, q).
p0_closure(m, q) :- p0_module_of(p, m), p0_reaches(p, q).
.decl p0_scope(m: symbol, s: symbol)
p0_scope(m, s) :- p0_closure(m, q), p0_module_of(q, s), s != m.
// Its rank: the files of its closure. A module ranks above every module in its scope.
.decl p0_module_rank(m: symbol, reached: number)
p0_module_rank(m, k) :- p0_module_of(_, m), k = count : { p0_closure(m, _) }.
// Its key: the engine's build and lowering switches, its name, and its closure's digests and edges, each in order.
.decl p0_module_key(m: symbol, key: symbol)
p0_module_key(m, @sha256(cat(b, "|", m, "|", ds, "|", es))) :- env("rerail.build", b), p0_closure_digests(m, ds), p0_closure_edges(m, es).
```

`p0_closure_digests` and `p0_closure_edges` are each one `join` over the closure: the same aggregate and functor
`source-language.rerail` builds its selector key with. A key reads digests, not other keys, so no recursion runs
through an aggregate.

**Two variants, as built (step 1).** A file is read with `core.rerail` first, so Rust's plan puts `core.rerail` in
every other file's scope, though no file names it. A program in T's shape declares `doc` itself and reads its files
without it: `calls.rerail` includes `python/modules.rerail` and `python/conditions.rerail`, which a Python policy
includes too. The same file is then two modules, so the rules plan each file reached in the variant of the seeds that
reach it, and a module's name says which: `core:<digest>+<digest>` or `bare:<digest>`. The `core` variant adds the
implicit edge to `core.rerail` from every file. `core.rerail` as a seed of its own declares `doc` too, but read with
itself first is itself, so it is planned `core`. A file seed is a module of the programs that include it
(`p0_seed_reaches(f, f)`), so a library file's graph holds its own module and a text is composed of them; a program
file of the run's own is a program's text and no module (§2), and `Graph::of` leaves it out. A text in T's shape is
not composed: the library's graphs hold the `core` variant alone, so its run derives its graph (the typing programs).

**Its cost.** A cold run of `bootstrap.rerail` over the Python pack's 537 texts (63 modules planned): 49 ms against 36
ms without the plan's rules; over the library's 64 files, 36 ms against 25 ms (spark2, one thread). A reused run's
check is 3–8 ms either way.

**The link order is canonical:** by rank, then by name. It depends on a module's files and what they reach, never on
the program that reached it.
- **The rank counts the module's own files.** Counting only what it reaches is not a topological order when a file
  includes a member of an include cycle: both reach the same files, and the tie is broken by digest. The spike found
  this on the Python library (E2: "topological: false"), and the reflexive count fixes it.
- **The printed program changes once.** In canonical order the library prints the same lines as in include order,
  ordered differently (E2). For django, node and the phase programs, a pass's `reads` list and the input list are
  ordered differently too. So program identities change once, as with any change of the engine's build. Neither list
  is read as an order, and T2 holds the suite's outputs to today's.
- **The order is per module, so composable.** Graphs are composed from their files' graphs without a run (§2), and a
  per-text order (today's include order) could not be composed. Keys are independent of the order in any case (E3).

### 2. The plan travels with each file's graph

Phase 0 rarely runs `bootstrap.rerail`. A seed's graph is found in four ways: kept in the process, read from its
image, *composed* from the graphs of the files a text includes (`phase0::composed`, no run), or derived by a run.
A test's program is a text whose graph is composed: one run per text was 9.6 k of the corpus suite's 125 k samples.
So the plan cannot be a per-text output:
- each `Seed::File` graph holds the plan rows of the modules its file reaches (`p0_module_of`, `p0_scope`,
  `p0_module_rank`, `p0_module_key`, restricted to its closure), derived in the run that derived it and kept in its
  image;
- a composed graph is the union of its files' graphs, so it holds the union of their plan rows. Keys are canonical,
  so the union is consistent.

The text's own statements are not keyed or kept: their walk is 1–6 ms a program (E1), and the program's own caches
(`Programs`, the prepared programs' store) are keyed by the whole graph already.

### 3. A module's lowering is a request in phase 0 (0104's two rules)

- **The key fact:** `p0_module_key(m, key)`.
- **The result:** `p0_module_lowered(key, image)`, `image` the digest of the module's image in the image store, as a
  new engine answer (`lower::ENGINE_ANSWERS`), answered in the run's request rounds as `load` and `file_digest` are.

```
.decl p0_lower_ask(key: symbol)
p0_lower_ask(k) :- p0_module_key(m, k),
    count : { p0_scope(m, _) } = count : { p0_scope(m, s), p0_module_key(s, sk), p0_module_lowered(sk, _) }.
```

A module is asked for once every module in its scope is lowered, so requests run in rounds in dependency order. The
engine answers a key once, as a load is answered once. Its native step:
1. Reads the members' statements off the tapes phase 0 loaded.
2. Reads the scope's modules from their images (`module_image::decode`).
3. Walks the clauses (`lower_module_in_written`).
4. Keeps the image (`keep_image`) and answers with its digest.

**As built (step 2): one wave, answered in rank order.** The rules ask for every module at once,
`p0_lower_req(key, rank, files, scope)`, with its files by path and its scope's keys. The engine answers the wave's
requests in rank order, so a module's scope (which ranks below it) is lowered first, and one pause of the fixpoint
answers them all rather than one pause a rank. A module that does not lower is not answered: the program that reads
it lowers it in its own place and reports the error there, not phase 0 for every program. The record's entry and
`Graph::holds` check that the store holds the image under its key (mapped, not read: the key names the content).
The store's writer keeps an image behind the run, so the process keeps each module it lowered or decoded, by store
and key (`rr::planned_module`). A library file or a phase program as a seed is a module of the programs that include it,
so its graph holds its own module and the template plans and lowers every one; a program file of the run's own is
no module.

**Precedent for rounds over answers.** `bootstrap.rerail` already derives requests through negation over an answer:
`load` ← `p0_resolved` ← `!p0_before` ← `p0_exists` ← `dir_entry`, the answer to `rr_dir_req`. The count over
`p0_module_lowered` is the same pattern.

**The module's parts are facts already.** `module_image::encode` writes a module's parts (`module_violation`,
`module_dep`, `module_once`, `module_set`, `module_written`, and so on) as `.about` relations of its image, beside its
rules. The image is the one block a module is kept as, read in place. The answer adds one row a module, and copies
none of the image into phase 0's facts.

**Reuse.** The bootstrap run is reused while its record holds (`reuse: true`). The answer records what it consulted,
as `analysis_history` does (`consulted_lookup`), with a `kept` check that the image is still in the store. Without
it, a reused run would never ask again for an image the store dropped.

**A graph holds only while its modules' images are there.** `Graph::holds` adds a look at each module's image (the
link reads them anyway). A graph kept in the process but missing an image in the current store (another test's store
in the same process) is derived again in that store, which asks for the lowering. The lowering is never done
anywhere else.

### 4. Helpers are named by content, so the link has nothing to decide

Today a helper is named by a counter (`fresh_name`: `rr_{stem}{next}`), and the link decides three things per program:
- **Renumbering:** a module's helpers move up by the sum of the counts of the modules linked before it.
- **Renaming clashing natives:** Python candidate natives that collide get a new serial.
- **Merging:** a helper whose `once_key` equals an earlier module's is merged into it. The key is its `once` stem,
  column types and rules as templates.

Each depends on the program's set and order of modules.

**Two tiers of names**, given when a module's walk ends (`lower::content_names`, spiked in E6). They are a rename of
the module's version, kept in its image:
- **A helper the walk may merge (`once`)** is named by its content: a Merkle digest of its `once` stem, its column
  types and its rules' templates, sorted. Each helper it reads is named by its own name, and itself as `@self`. The
  `once` stem matters: an aggregate's names its readers' heads (`agg h1,h2`), because demand narrows the helper by
  its readers. Naming by the bare stem merged 17 of Python's helpers that the walk keeps apart (E6, first run).
- **Any other helper** (`ctx`, `t`, `earlier`, `precedes`, `next_sibling`, `cover`, `up`, ...) is named by its module's
  key and its local name. Today's lowering never merges these, and later passes may rewrite one for its reader. The
  name is still free of the link's order: a module's key and its own walk fix it.
- **Helpers that read each other in a cycle** take the second tier too. Naming a cycle by content is graph
  canonization, and a wrong canonical form merges different helpers. There are none today (E6).
- **Python candidate natives** take the first tier, by the native's content.

The findings' counter (`PendingFindings::next`) and the `must`/`may` rule positions (offset by `appended`) are
numbered by module the same way, so nothing in a module depends on what is linked before it.

**The link is then a union, written as rules:**

```
linked_rule(t, r)   :- text_module(t, m), module_rule(m, r).
linked_decl(t, d)   :- text_module(t, m), module_decl(m, d).
linked_fact(t, f)   :- text_module(t, m), module_fact(m, f).
...
link_conflict(t, n) :- text_module(t, a), text_module(t, b), module_decl(a, n, c1), module_decl(b, n, c2), c1 != c2.
```

These rules run in the run that reads the program's graph, not in a run of their own. That is 0246 §2: linking makes
the run's base. **The engine specializes the shape:** a union over a set of keys of relations kept in blocks is a view
over the module images (0244), their string tables interned and their ids offset, with no row of the union produced.
`link_conflict` (and the name checks of § Collisions) are the only rules that produce rows, and they are empty.

**What the spike found of the fields** (`append_fields`, E7):
- No library has a declaration, native or kind set two modules give different values, so "a later module's
  replaces" never decides. It becomes the `link_conflict` error.
- Each input is stated by one module, but for one input in four packs, and a few relations take facts from more than
  one module (`extensible`, 0263).
- Settings and demands come from several modules. Their lists are taken in the canonical order, which is per
  module, so even an order that is read stays deterministic. T8 holds that no reader depends on it.

**For a modular text,** `Text::lower_version(true)` becomes:
1. `check_names` and `closed_program`, as now.
2. The text's own module lowered against its modules (`lower_module_in_written`), each read from its image by its
   `p0_module_lowered` digest.
3. The link's union, then the tail and `contexts::passes` over it.

A plain text keeps the whole lowering, and `bootstrap.rerail` itself, lowered before phase 0 runs, does too.
`lower_library` (packs) takes the same union, and `library_modules_with_files`' graph code goes.

**What the names change.** Every helper's name in every printed program changes once, so program identities change
with them. That is the one-time cost the canonical order (§1) already has. Four test files name numbered helpers
(22 literals). In exchange, an edit of one module no longer renumbers the helpers of every module linked after it,
so their printed rules and digests stay the same (0225's incremental work).

**As built (step 3).** Every walk names its helpers at its end (`Lowerer::name_helpers`), so a library module, a
program's own module and a whole text are named alike:
- A `once` helper is `rr_h_{stem}_{128 bits}`. Its key holds its `once` stem, its column types, its rules' templates
  sorted, every header row that names it (any `.about` row with a cell of its name: a qualifier, a tape column, a
  native, a layout, an answer, a request's readers, ...), its facts, and whether it is an input or an output. A field
  added later joins the key with no change here (Collisions 4, T7).
- Any other helper, and a cycle of several, is `rr_m_{stem}_{128 bits}`, of its module's digest and its walk's
  name. The module's digest is its own walk's content (its rules, declarations, header rows, facts, inputs and
  outputs), not its plan key, so the walk names without its caller passing a key, and two modules share these names
  only where they are one.
- A Python candidates native is `rr_h_python_candidates_{128 bits}`, of the native's content, where it is made.
- Helpers of one content name in a walk are one, and the later's rules and declaration are left out. Their keys are
  compared byte for byte, so a digest's collision is an error (Collisions 1, 2). This replaces `merge_helpers`,
  `once_key` and its `DefaultHasher` template: no merge reads a 64-bit hash.
- A template is length-prefixed fields, read back by `lower::template_nodes` (Collisions 3).

The link then decides no name. A helper of a name linked before is that one, and the module's rules and declaration
of it are left out. No renumbering, no native renamed, no key compared. The `must`/`may` positions are moved past the
rules a merge leaves out, in the walk and in the link, where before they kept the index of a dropped rule's place. The
walk's counter is spent at its end, so the findings' helpers are numbered from one whatever was linked. A module's
image keeps its helpers by name (`module_helper`); `made`, `next` and `once` are no longer kept, and the image key's
format is v3.

The code that found a helper by its old stem (aggregate demand's helpers, the widening of cyclic contexts, hiding's
contexts) reads it with `lower::helper_stem` and `lower::holds_helper`, one parse of the two forms. T6 holds for the
Python and Rust packs: a library's modules, linked by name, have the whole list's rule lines (each module-named
helper by its stem, and the walk's numbered variables, which a template takes by slot, aside), the same content
names and as many module names.

**As built (step 4, stage A: the plan's modules, the union's checks).** A modular text keeps its graph's plan
(`Read::Modular`'s `plan`, `rr::plan_of`): its modules by rank, then name, each as its lowering request names it.
`Text::lower_version(true)` reads each module from its image by its plan key and lowers the text's own statements as
one more module against them (`linked_from_plan`); a module phase 0 could not lower is lowered there, so its error is
the program's. `lower_library` (packs) takes its modules from the plan of the library files as seeds (a library file
is a module of its own, step 2), merged over their graphs. `library_plan`, `module_key` and
`library_modules_with_files` are gone. T3's reference plan is in its test. A plan key is the store's alias with the
image format and the lowering switches (`planned_key`), as `module_key` had them.
- **The closed relations check (0263)** moved into the module's lowering: the request names its scope's files
  (`p0_scope_files`), and `lower_planned_module` checks each file of the module against their declarations.
- **The union's checks.** `link_versions` refuses a relation two modules declare with other column types, make
  natively otherwise, or give other kind sets (`lower::link_conflict`), and a helper name whose modules give it other
  rules, compared by their sorted templates (`helper_conflict`). A content-named helper is declared with its first
  reader's column names, so a declaration is its column types.
- **A module reads its scope's bound families' declarations.** Lowered against git's modules, a module failed with
  "bound-family: undeclared relation git_object_request": `bound_family::lower` looked up an inherited family's
  relations in the module's own version, which leaves its scope's declarations out. It reads them from the scope's
  seeded header too. The whole lowering never met this, as the family's declaration was in its one module.

T1 `a_program_linked_from_its_modules_is_its_whole_lowering` (two programs of every pack and of the phase programs; the
spike runs them all) and T8 `a_link_of_modules_that_clash_fails`. Stage B is the union as a view over the images
(below), which removes `link_versions`' copying and `AppendModule`.

**Stage B, designed: the union as a version over the module images.** A version's ids are one space per stack:
a delta's atoms, relations and rules number past its parent's, and every handle reads its ids from its image's
nodes as they are. Module images are encoded apart, each numbering from 0, and the same relation (`kind`, a helper
named by content) has a different id in each. So the union is a new form of version, `Version::Union`:
- **Its names are a header version**, a base (or an overlay of one) with no rules: every text and relation of every
  part interned, and the program's fields (declarations, facts, inputs, outputs, `.about` rows) as the link states
  them (`append_fields`, the first part's fields copied whole, a merged helper's first declaration kept). Its ids are
  the union's; `text`, `find`, `decl`, `facts`, `inputs` read it.
- **Its rules are the parts'**, each part a version (a module's base, as its image is read where the store holds it),
  with the ranges of its rules the link keeps (a merged helper's are left out, `moved_past`) and an `IdMap`: the
  union's id of each of the part's atoms and relations. Part `k`'s rule `j` is rule `base_k + j`; `heads` and
  `readers` map the union's relation to the part's and back.
- **Handles carry the map.** `RuleRef`, `AtomRef`, `LitRef` and the term iterators hold the part's `IdMap` (none for
  every other version), and map each atom or relation id they read: a relation, a string or call constant, a branded
  atom, a variable's name, a join's separator, a kind set, a matcher, a capture. A builder copies a mapped rule by
  text (`Encoder::map_val` maps first), as it copies any rule of other ids.
- **Edits stack on it** as on any version: an overlay's ids number past the union's. A fold makes a base of it.
- **A program's union extends its library's.** The library's union (its header, its parts with their maps, and the
  lowerer's knowledge of them) is made once per set of module keys in a process, and a program's union is the same
  parts and its own module, its header an overlay of the library's with the own module's texts and fields. So a
  program interns and states only its own module's names, and copies no rule.

`link_versions` makes the union in place of the first module copied and the rest appended; `AppendModule` and the
appended list of `lowering_tail_on` go, and a scope's program (`link_seeded_version`) is the union of the scope's
header and the program's own module.

**As built (step 4, stage B: the union).** `Version::Union { header, parts, end, .. }` and `UnionPart { version, keep,
map }` (`program_image/version.rs`), `IdMap` on every handle (`handles.rs`), and `Encoder::map_val` mapping a part's
ids before it looks a text up. `Lowerer::link` returns the module as kept (`LinkedPart`: its version, the ranges it
keeps, the helpers it merged), never a copy. `lower::link_library` makes a `LinkedLibrary` (the header, the parts,
the lowerer's state); `LinkedLibrary::with(own)` adds a program's own module, its header an overlay of the library's.
`rr::linked_from_plan` keeps one `LinkedLibrary` per image store and set of module plan keys in the process
(`LinkedLibraries`), so a program interns only its own module's names. `link_seeded_version` is `Linked::union`: the
scope's header and the own module. `AppendModule`, the appended list of `lowering_tail_on` and the fold at the end
of `link_library_versions` are gone; a caller that keeps an image folds (`lower_library`).
- **A stored run keeps a union** as kind 3 of `pipeline_run_version` with a row of `pipeline_run_union_part` per part
  (its version's place and its kept ranges); a reader finds each part's map again by text (`UnionPart::found_in`).
- **A view's ids are its parent's**, so `Version::rule_end` of a view is its parent's: through the top image it was
  the header's (no rules), and an overlay on a view of a union numbered its rules over the union's. The SQL suite
  found it (every program there is an edit of a view of its union).
- **Raw readers map.** A rule's template (`template_into`, `rule_reads`) reads ids through `RuleRef::atom_of` and
  `rel_of`.

Tests: `a_linked_library_is_a_union_over_its_modules_images` (each part is its module's version, uncopied; the union
and its fold have one digest and the same rules in order; a view of it and an overlay on the view number past it),
`programs_of_one_library_share_its_union` (two programs of one library's modules make its union once). T1, T6 and T8
pass on the union.

**Why the tail stays Rust.** The tail's whole-program questions (which core relations are read, which kind sets are
read) belong to preparation's fact spaces (0246 §3, 0225 §5).

**A module depends on its scope's interface, not its scope's rules.** `Lowerer::seed` and `seeds` read from a scope:
its header less its helpers (`Given`), its lattices, three-valued relations, `.violation`s, dependency edges
(`deps`, which a clause's context reads: `reaches_heads`), printed demands, bound families and column types as
written. E9 lowered every module against its scope lowered from its statements less its rules: 170 of 213 modules
gave the same image. Each of the other 43 reuses a declaration its scope's *lowering* made, never its dependency
edges or rule bodies: a kind set helper (`rr_kind_class_definition`, `rr_inside_rr_kind_…`), the `tape` input,
`rr_embed_req`, `rr_dir_req`/`rr_file_req`, `rx_span`. So a module declares those itself where it uses them, as it
does against a header-only scope. Their names are their content already, so the union holds each once. A module's
key can then name its scope's interface digests, not its scope's files: an edit of a rule body that leaves a module's
interface unchanged lowers that module again, and no module that includes it.

**As built (step 5).** The lowerer keeps the declarations its clauses' lowering makes (`uses`: `tape`, `rx_span`, a
one-kind set, a demand's request, the file system's request), and `unseed` keeps each as the module's own though its
scope made it too. The inputs a module's lowering adds follow its stated ones by name, so their order does not depend
on which module made one first. In a key's rows an unnamed made helper is `@helper <stem>`, and a helper that reads a
module-named one is module-named. T9 is `every_module_lowers_alike_against_its_scopes_headers`: every module of every
pack's library and of the phase programs lowers to its image against its scope's headers alone.

A file's interface digest is `rr::interface_digest`: SHA-256 over its statements but its rules with a body and its
includes, each length-prefixed. It is the file system's answer `file_interface(path, d)` (`core.rerail`,
`rr_interface_req`), beside `file_digest`: a request keyed by its path, which the lowering derives from the literals
that bind the path. Written as an engine answer with its request in `bootstrap.rerail`, it was answered but its rows
did not reach the key's rules. `p0_key_digest` names a module's own files by their bytes and its scope's by their
interface, and `p0_closure_digests` and `p0_edge_text` read it. The run's record keeps each answer, so a reused phase 0
run checks it again (`World::observation`). The image key's salt is v4. Test `a_modules_key_names_its_scopes_interfaces`:
an edit of a rule's body in a module's scope keeps the module's key; an edit of a declaration or a fact there changes it.

### 5. The template holds every module

`test_store::fill` already gives phase 0 every library file and phase program as seeds, in one run. With §3, that run
asks for every module, so the template's store holds every module's image, and every test's copy of it finds them.

### 6. The tail, by module

After the link, a program's lowering is the tail. Per tail run (Python, 152 runs, `RERAIL_SPIKE_PROFILE`, spans
read by `clock_gettime`: see below):

| Step | ms | Reads | Where it goes |
|---|---|---|---|
| `check_given`: cycles, caches, answers | 5.0 | SCCs of the program; a cache's two rules; a request's answer | SCCs compose over modules: per module, and per program only through E8's relations. The cache and answer checks are per relation |
| guest context passes (0597), crossing cycles | 4.2 | the module declaring a context, and the scope's rules its exports reach | upward: at the module that declares the context (E11) |
| sources on request (two rounds) | 3.2 | what the program reads | downward: per program, as rules |
| aggregate demand | 2.8 | a helper's readers (in its module) and strata | per module; the strata check per program only through E8's relations |
| `lower::link_versions` | 2.5 | every module | gone: the union view (§4) |
| kind sets | 2.1 | which sets a rule reads | downward: three rules (E10), per program |
| span patterns | 2.0 | the columns' spans, from the scope | upward: per module |
| append a module | 1.9 | every module | gone: the union view |
| order the nodes compared | 1.7 | a rule and its relations' column types | local: per module (its `rr_ordered<k>` counter named by content) |
| findings, core declarations | 2.2 | a `.violation`'s rules; what rules read | local, or a union of each module's |
| the rest (unit, literals once, nullary, axes, widening, ...) | ≈2.4 | a rule | local |

**The tail's time outside its passes was the profiler's clock.** A first profile showed 4.6 ms a run in the tail
outside every pass. Timed directly around each `program()` call, the passes took 3.518 s against their spans' 2.636 s.
On Linux the spans read `getrusage(RUSAGE_THREAD)`, which gives the running thread's time as of its last scheduler
update. A span's close then calls `getrusage(RUSAGE_SELF)` (`memory::peak_sampled`), which brings it up to date, so
each span lost the time since the last update to its parent. The profile now reads
`clock_gettime(CLOCK_THREAD_CPUTIME_ID)` on every Unix, as it did on macOS. The tail's own time is then 0.03 ms a
run, and the passes' spans hold 3.502 s of the clock's 3.518 s. Linux profiles taken before charged part of each
span's time to its parent.

**The guest context passes (E11).** Python's per run: the instances written 2.0 ms, the plan 0.7, the program's SCCs
0.7, the settings read twice 0.4, the passes' own 0.4; and 0.4 ms of rule 8's report, made only in a profiled run
(`profile::decision`), left out of the table. The passes rewrite the whole program in every run whose library declares
a context (72 of 76 Python programs, every Django and Python-main one), though none of those programs declares or
completes one. A program's link holds its library's pass rules as the library's link alone makes them, but for 3
(Python) and 6 (Django) that differ only by `order the nodes compared`'s counter (`$rr_ord28` for `$rr_ord26`), which
this section names by content. Vue is the one pack whose programs declare passes (15 of 22): there the program's own
module makes 921 instance rules and renames 185 of its own, from its rules and the scope's rules its exports reach.
So the passes are upward: made once, at the module that declares the context, over its scope's kept rules; and the
check for a cycle through two passes is the SCC check's, per module, and per program only through E8's relations.

Each tail step is one of four kinds, and each kind has its place:
- **Local** (a rule or a relation, with declarations): at module time, in the module's image.
- **Upward** (a scope's results into a module: three-valued closure, span columns, SCCs): at module time, over the
  scope's kept results.
- **Downward** (from what the program reads: kind sets read, sources on request, demand): per program, as rules over
  the union's rows (E10), maintained by delta (0225) as the program's own module changes.
- **Across modules**: a relation that a module other than the reader's scope defines or adds rules to. E8 counted
  them: in all libraries 12,459 of 12,934 rules (96%) read nothing but their module and its scope. The rest sit below
  `layout_at` and `token_text_ask` (JavaScript, TypeScript, Node), `node_module` (Node), `php_dynamic` (Blade),
  `cc_decision` and `cc_name` (complexity), `rr_embed_req`, `rr_dir_req`/`rr_file_req`, `c_include_dir`/`c_tu`
  (phases) and `cmp_same_ask` (Python). Only those relations' SCCs, three-valued closures and strata are completed per
  program.

### 6a. The tail by module, spiked (E12–E16)

The tail is a list of named steps (`rr::TailStep`; `rr::TAIL` is its order, `rr::downward_steps()` the downward steps,
`rr::tail_steps` runs any list), so its parts run apart. Spike `spike_0639_tail_by_module` (ignored), on spark2, over
all 17 packs' libraries and the phase programs, and up to 6 programs a pack (96 programs):

| | Question | Result |
|---|---|---|
| E12 | The *module steps* (`TAIL` less `KindSets` and `SourcesOnRequest`), run on each module linked with its scope, give together what they give on the whole library? Lines compared with variables renamed by place | yes in 16 libraries. Vue: `vue/component`'s own tail keeps host copies of 11 relations (`vaccessor`, `vmacro`, …) that the whole leaves to the context `script`. `vue/script` declares that context again with more exports, which reach them. No host rule reads the copies, so `Prune` drops them when the plan is made. Nothing else differs, and no context instance is missing |
| E12b | The module steps twice, the second over the first's output: as once? | yes in all 16 |
| E13 | A program's tail with the module steps first, then the downward ones, then the steps that follow them in `TAIL`: `TAIL`'s program? | yes in all 93 compared. 3 more fail alike in both orders: `eslint-curly`'s and `eslint-plugin-vue-no-child-content`'s known inline failures, and `phpcs-generic-php-deprecated-functions`, whose facts file the spike's module collection leaves out (it is included from `../packs/php/`; the product's parse holds it, E1) |
| E14 | A strongly connected component holding relations two modules give rules to? | none in any library. Relations with rules from two modules are E8's (`layout_at`, `token_text_ask`, `cc_name`, `cc_decision`, `php_dynamic`, `rr_embed_req`, `rr_dir_req`, `rr_file_req`) and content-named helpers |
| E16 | The steps after the downward ones change only relations that the downward ones or the program's own module changed? | yes in all 93 |

E15 and E16's times, per program on one thread (ms; the module steps here over the whole union, as today):

| Pack | `TAIL` | `KindSets` | `SourcesOnRequest` | the steps after them | Largest others |
|---|---|---|---|---|---|
| django | 77.4 | 5.2 | 7.3 | 26.8 | ContextPasses 15.8, Checks 15.4, AggregateDemand 5.2 |
| react | 37.6 | 1.8 | 8.9 | 10.3 | Checks 4.2, AggregateDemand 3.4, ContextPasses 3.0 |
| python | 30.8 | 2.0 | 2.6 | 10.0 | ContextPasses 6.6, Checks 4.8, AggregateDemand 2.7 |
| javascript | 19.3 | 0.9 | 4.8 | 5.3 | Checks 1.9, AggregateDemand 1.7 |
| vue | 19.1 | 1.0 | 4.8 | 4.1 | ContextPasses 3.6, Checks 1.4 |
| phases | 16.5 | 0.6 | 2.0 | 7.4 | Checks 4.5, ContextPasses 1.9 |

**What step 6 builds.**

1. **A module's tail at module time.** Phase 0's lowering request (§3) runs the module steps over the link of the
   module's scope (their tailed images) and the module, and keeps in the module's image the rules that the run
   writes and that no scope image holds. That covers the module's own rules, tailed, and what its tail makes of
   its scope's: a context's instances made at the module that declares it (E11), and a module's own copy of a
   relation it renames. E12 and E12b are why this composes. The parts' union is the whole library's tail (up to the
   dead host copies `Prune` drops), and a scope image the steps run over again comes back as it was.
2. **A second key, the tail's.** The module steps read the scope's rules themselves, not only its interface:
   a context's instance is a copy of a scope rule's body, and the three-valued closure and the span columns read
   the scope's rules. Step 5's key (§4) names the scope by interface, so a body edit in the scope keeps the lowered
   image. The tailed image needs its own key: the lowered image's digest and the scope modules' tailed digests.
   A body edit in the scope then re-runs only the module steps of the modules that read it. No lowering runs
   again. Kept as `p0_module_tailed(key, image)` beside `p0_module_lowered`, answered by the engine.
3. **A program's tail.** The link of the tailed library images, and the program's own module tailed in its scope
   (1. on the program's module). Then `KindSets` and `SourcesOnRequest` over the union, then the steps after them
   run on what changed alone (E16): each is a local step, so it reruns through `ProgramPass::again` over the
   downward steps' edit (decision 0393; `Pipeline::again`). Of the checks (`finish_checks_given`), the program
   runs only those of the components that hold the own module's relations or the downward steps' changes (E14: no
   component crosses modules).
4. **The downward steps as rules (T10).** `KindSets` becomes E10's three rules over the union's reads, evaluated
   and kept by delta as the program's own module changes (0225 §5). Their input is a relational view of a
   program image (decision 0662), the first analysis written that way. `SourcesOnRequest` keeps its `again` until
   decision 0662's view carries what it reads.
5. **The steps' places declared.** Each tail step says whether it is local, upward, downward or across
   (decision 0662), so the module/program split above reads the declarations. It does not keep a list of its own:
   `DOWNWARD` becomes the steps that declare `Downward`. T11 and T13 hold the declarations to what the steps do.

**Tests, before the code.**
- **T10.** E10 as a test: the kind set rules take out what `KindSets` takes out, every pack.
- **T11.** E12 and E12b as a test: every library's modules, tailed at module time, unite to the library's tail
  after `Prune`; the module steps over a tailed scope leave it as it was.
- **T12.** E13 as a test, on the product path: every pack program, linked from tailed images, plans to the program
  it plans to today after `Prune`.
- **T13.** E16 as a test: the steps after the downward ones change nothing but the downward steps' and the own
  module's relations.
- **T14.** A rule body edited in a scope file re-tails the modules whose tail reads it (their tail key changes), and
  lowers none again (their lowering key does not).

**Use edges.** A relation a module reads that a later module defines is the rules' function-local import: lowering
needs only its declaration, so the plan stays a DAG; evaluation has the cycle. Phase 0 has one such edge today:
`p0_phase` (a `<phase>__<export>` declaration loads the phase program as a rule file of the run, not into the module's
scope). An explicit `#use` would add the general case: the file joins the program's link set and not the module's
scope, so two files that need each other's rules stop being one module.

## Spike (2026-11-23)

`spike_0639_programs_linked_from_modules` (`tests_rr`, ignored: `RERAIL_SPIKE_PACKS`, `RERAIL_SPIKE_N`), run on
spark2, `RERAIL_THREADS=1`, over every pack and the engine's phase programs.

| | Question | Result |
|---|---|---|
| E1 | A program linked from its library's modules and its own module prints as `parse_version` | 1,291 of 1,291 identical, helper names included. 7 fail inline (below), and so are not compared |
| E1 | Its cost on one thread | The two Python packs (889 programs): 64 ms a program inline, 39 ms on the module path (own module 6 ms; link, tail and passes 33 ms). The others (402): 48 ms and 27 ms (2 ms and 25 ms). Each library's modules lowered once: 0.8–1.0 s over all of them |
| E2 | The library's lowering in canonical order against include order | the same lines in another order; django, node, phases: also a pass's `reads` and the input list ordered differently |
| E2 | Linked from modules in canonical order against lowered whole in canonical order | identical, every pack |
| E2 | The canonical order is topological | yes, with the rank counting the module's own files; no, without (Python) |
| E3 | A module lowered against its scope in two orders | one image, every module of every pack's library (none differ) |
| E4 | The images | Python: 63 modules, 1.31 MB, decoded in 3.3 ms; the other packs 0.09–0.66 MB |
| E4 | Helpers that read themselves | 0–10 rules a pack: content-addressed names (§4) must name them by their module too |
| E5 | The plan as rules (§1) against the Rust plan | modules, scopes and ranks equal, every pack; keys distinct, and the same with the facts stated in reverse |
| E5 | A run of the plan's rules | 50–90 ms each, as a standalone corpus run through the tests' helper: mostly the run's fixed cost, so not a cost of rules inside an existing run |
| E6 | Content-named modules united, against the content-named whole lowering | the same rules, every library (the second tier aside, whose names carry the module) |
| E6 | Against today's link, helper names aside | the same rules and the same helper count in all 17 libraries (Python 204, python-main 79, phases 68, ...), with the `once` stem in the key. With the bare stem: 17 more merges in Python. The rules only the union shows differ from today's by variable names: one rule by template digest, which the union keeps once |
| E6 | Helpers in cycles of more than one | none |
| E7 | Fields two modules give different values | none, every library |
| E7 | Shared inputs; relations with facts from several modules | one input in four packs; `cc_boundary`, `api_known`, `node_global`, `seen_ask` |
| E4 | A module's image holds its file's path or name | no image does (some hold a stem that is also a word of the file, such as `annotation`) |
| E8 | Rules whose every relation below them comes from their module and its scope | 12,459 of 12,934, summed over the 17 libraries; django, next, php, rerail, rust, sql all; Blade 83 of 113 the fewest |
| E8 | Relations with different rules from more than one module | `php_dynamic`, `cc_decision`, `cc_name`, `layout_at`, `token_text_ask`, `rr_embed_req`, `rr_dir_req`, `rr_file_req`. The 73 `extensible` declarations add facts across modules far more than rules |
| E9 | A module against its scope lowered without its rules | 170 of 213 the same image; the 43 others reuse a declaration the scope's lowering made (kind set helpers in 10 files, `tape` in 8, `rr_embed_req` in 3, `rr_dir_req`/`rr_file_req`, `rx_span`) |
| E10 | The tail's `KindSets` pass against three `.rerail` rules over the same input's reads | the same sets taken out in all 17 libraries (django 10, Python 7, Vue 7, ...) |
| E11 | The guest context passes: a program's link against its library's link alone | no program outside Vue declares or completes a pass; the library's pass rules the same, 9 (Python, Django) by an `rr_ord` counter alone; Vue's 15 programs with passes make them in their own module (921 instance rules, 185 renamed). Python: 4.2 ms a run, and 0.4 ms more of a profiled run's report |
| — | The link and the tail per run, by span | `link_versions` 2.5 ms, the tail 28.5 ms (§6's table) |

The 7 inline failures fail the same without this change: `ruff-perf403` (refused at reading, open items),
`eslint-curly`, `eslint-no-object-constructor` and `clippy-redundant-field-names` ("message: no column"),
`eslint-one-var` (`ordered` on a `.decl`), and `next-no-page-custom-font` and `eslint-plugin-vue-no-child-content`
(`match_all` over an unbound node).

What the spike leaves: the link, tail and passes are most of a program's lowering (33 of 39 ms for Python). §6 places
each step: the link goes with the union view, most of the tail moves to module time, and what stays per program is
downward rules over the union's rows and the few relations E8 lists. E11 places the guest context passes at the
module that declares the context.

## Collisions

A content name says "these are one". A collision makes two different things one, and the union then merges them
silently. Each place a digest stands for content, what can collide, and what is done about it:

1. **A digest's bits (helper names, module keys, rule identities).** The spike names helpers by 64 bits of SHA-256. In
   a program of about 1,000 helpers, the chance of a collision is about 1,000² / 2⁶⁵, around 10⁻¹⁴. The image store
   is shared across projects, so the population is every helper ever kept. **Done:** names carry 128 bits, keys and
   rule identities 256. The union checks that a name means one thing: `helper_conflict(t, n)` holds where two modules
   define `n` by different rule digests, and a non-empty conflict fails the run. That is a join over rows the images
   hold, so a collision is an error, never a wrong answer.
2. **Today's merge key is weaker than that.** `template_of` hashes each rule with `std::hash::DefaultHasher`: 64 bits,
   whose algorithm the standard library may change between releases. A collision merges two different helpers today,
   unchecked. Its keys are not kept across builds, so only the probability is the risk. **Done:** `once_key` takes
   the SHA-256 template (`template_into`), and the walk's merge checks equal templates on a hit.
3. **The encoding is ambiguous.** Two different rules could write the same bytes. Each node writes its kind, its size
   and its value, and the names and texts end in `0xff`, which UTF-8 never holds. The key's stem and column types are
   separated by `0xfe`. **Done:** the encoding is written down as length-prefixed fields, and a test round-trips a
   template's bytes to its rule.
4. **What the key leaves out.** A helper means its rules, its column types, and any field that names it: a native,
   an answer, a tape column, a qualifier, a layout, a demand, a gate. The spike's key holds only the first two. Two
   helpers with the same rules and different fields would merge. **Done:** the key holds every field row that names
   the helper, and T7 holds that every field naming a relation is in the key.
5. **A name in a rule means something else in another module.** A template names the relations a rule reads by name,
   so two modules' helpers merge if they read different relations of one name. Names are global only while no two
   modules declare one name differently. E7 found none, but nothing enforces it at the link. **Done:** `link_conflict`
   (§4) fails the link. With it, a name is one relation, and naming by name is sound.
6. **The same content meant to stay apart.** Equal rules are one relation, but a later pass may rewrite a helper for
   its reader: aggregate demand narrows by readers, and `WidenCyclicContexts` widens a context. Merging two readers'
   helpers would then narrow or widen both. **Done:** only `once` helpers are named by content, with the walk's `once`
   stem (the readers' heads for an aggregate). Every other helper is named by its module. E6 shows this merges exactly
   what the walk and today's link merge.
7. **Cycles.** A helper's name reads the names of the helpers it reads, so a cycle has no first name. A self-reading
   helper names itself `@self`, which is exact. A cycle of several would need a canonical form of a graph, and a wrong
   one merges different helpers. **Done:** a cycle of several takes the second tier (its module's key). None exist
   today.
8. **A helper's rules added after it is made.** `once` records a helper's rule count when made, and today's merge
   compares only those first rules, keeping the merged helper's later rules under the kept name. A reader of the kept
   helper would then see rules another module added. The content name is taken over all of the helper's rules, at the
   walk's end, so helpers with different later rules are two. No helper in the libraries has later rules: E6's merges
   equal today's.
9. **Module keys leave paths out.** Two files of the same bytes at different paths are one module. That is right only
   while a module's image does not depend on its path. **Checked** (E4): no image holds its path or file name. T6
   keeps it so.
10. **Rule identities in the union.** The union keeps a rule once by its digest: its head's name and its template.
    Two identical rules of two modules become one, as the tail's `DropRepeated` does today. A rule's place (its
    file and line, for a finding's message) is not in its template. A duplicate's place is lost only where the rule
    is the same everywhere, which is the case today too.
11. **Names of the two tiers and the tail's helpers.** Content names (`rr_h_<stem>_<32 hex>`), module names
    (`rr_m_<stem>_<32 hex>`) and the tail's counter names (`rr_<stem><n>`) are three disjoint forms. A printed program
    read again declares its helpers, and lowering it makes the same content names: the same relation, not a clash.

## Tests, written before the code

- **T1.** E1 as a test: for every pack program and every phase program, the module path gives today's rules and
  helper count, helper names aside.
- **T2.** The corpus suite and the rr tests pass, and their outputs are today's.
- **T3.** E5 as a test: phase 0's plan equals the plan over the library's files, and its keys do not change with the
  facts' order.
- **T4.** A cold run of a Python program on a copy of the template asks for no module lowering (`p0_lower_ask`
  answered from kept results), and a store with one image dropped asks for that module alone.
- **T5.** A reused bootstrap run whose module image is gone asks for the lowering again (§3, reuse).
- **T6.** E6 as a test: content-named modules united equal today's link, helper names aside, with the same helper
  count; no image holds its path.
- **T7.** Every field that names a relation is in a helper's content key (Collisions 4), and two helpers differing
  only in such a field get two names.
- **T9.** E9 as a test: every module lowers to the same image against its scope's headers alone.
- **T10.** E10 as a test: the kind set rules take out what `KindSets` takes out, every pack.
- **T8.** `link_conflict` and `helper_conflict` fail a link built to clash, and are empty on every pack. The
  suite's outputs do not change when the settings' and demands' module order is reversed.

## Steps

1. §1 in `bootstrap.rerail` and the plan rows in `Graph` and its image (§2). T3. **Done**: T3 is
   `phase_0_plans_the_modules_as_the_library_lowering_does`; Rust's plan is `rr::library_plan`, which step 4 removes.
2. §3: the engine answer, its reuse check, and `Graph::holds` over images. T4, T5. **Done**: T4
   `phase_0_lowers_each_module_its_store_does_not_hold`, T5 `a_graph_holds_only_while_its_modules_images_are_held`.
3. §4's names: `content_names` in the walk's end, `once_key` on SHA-256, the findings' and rule positions' counters
   per module. T6, T7. **Done**: T6 `modules_named_by_content_link_to_the_whole_lowering`, T7
   `a_helpers_content_name_holds_every_field_that_names_it`, Collisions 3 `a_templates_bytes_read_back_as_its_rule`;
   `once_key` is gone rather than moved to SHA-256, as the content name is the merge.
4. §4's link: the union rules, the engine's view over module images, the conflicts. Single programs and
   `lower_library` onto it, and `link_versions` and the Rust plan removed. T1, T2, T8. **Done**: stage A (the plan's
   modules, the union's checks) and stage B (`Version::Union`, `lower::link_library`); `link_versions`,
   `AppendModule` and `library_plan` are gone.
5. §4's interface: a module declares the lowering-made declarations it uses; its key names its scope's interface
   digests. T9. **Done**: T9 `every_module_lowers_alike_against_its_scopes_headers`, and
   `a_modules_key_names_its_scopes_interfaces`; the digest is the file system's `file_interface`.
6. §6: the local and upward tail steps at module time; kind sets, then sources on request, as rules per program.
   T10. **Spiked** (§6a, E12–E16) and specified: the module steps at module time under a tail key
   (`p0_module_tailed`), the program's downward steps over the union and the steps after them on their edit, kind
   sets as rules over decision 0662's view, the steps' places declared. T10–T14.
   **Built (2026-11-24), T10–T14 passing:**
   - Each module's tail at module time, under `p0_tail_key` / `p0_tail_req -> p0_module_tailed`; a program links the
     tails and runs `program_steps` (T11, T12, T14).
   - Module-time decisions that a reader decides moved to program time (`Place::Across`), each at its `TAIL`
     position before the downward steps: core.rerail's fallback declarations and inputs (`CoreDecls`, stripped from
     the kept image), `within` where read (`WithinRead`, split from `BuiltinsOut`), a made context's `.pass` rows as
     one (`PassesOnce`), and a host's copies a reader's instance holds (`HostCopiesOut`). Run after the downward steps,
     `SourcesOnRequest` widened a dead host's copy (Vue's no-dupe-keys).
   - T13 restated: each non-across step after the downward ones changes only its edit (the downward changes, the own
     module's heads, and what earlier across steps changed). `AggregateDemand` reads across modules.
   - Tests that plan on pool threads bind the test's store on the test thread first: two phase-0 runs at once in one
     process store wait on its atoms lease and corrupted kept facts.
   - `DOWNWARD` is `downward_steps()`, the steps that declare `Place::Downward`.
   - T10: the kind set take-out as rules (`rr/kind-sets.rerail` over `rr/view.rerail`, decision 0662's view, names
     `img_`) takes out the same sets, undeclarations and facts as `KindSets` in every pack.
   - The program's checks are of its own components (`cycles::check_within`) where every step's changes are read off
     its overlays and views (`rr::changed_since`). `cycles::check` is 15.0 of django's 15.5 ms of checks. A step
     that makes a new base (`KindSets` rewriting rules, `AggregateDemand` placing demand) leaves its changes unread,
     and the program is checked whole: in the packs most programs still are.

   **Left of step 6:**
   - A derivation's changes, read off it: the pass manager records, for a derived base, the heads of the rules it
     wrote rather than kept, so `Delta::of` gives them and the checks narrow for every program. Writing `KindSets`
     and `AggregateDemand` as edits instead was tried and dropped: stating read-through fields on an overlay repeats
     list settings (`.cache` named twice), and the deeper overlay chains broke the `program_image` tests' forms.
   - The steps after the downward ones over their edit alone (`Pipeline::again`): the program has no kept run of
     those steps to be again of (its modules ran them apart), so the last run is the union as its own output, which
     the local steps leave as it is (T11's second half).
   - `KindSets` as a rules pass in the Pipeline (decision 0662 §5.1): the view read into a run in place and the
     pass's plan kept with its image. No in-process evaluator exists apart from a corpus run (50–90 ms fixed, E5,
     against `KindSets`' 2 ms), and a second one would break S003. The view as an engine input is the work.
7. §5 checked by T4 on the template, and the template's fill time measured.
8. The cold run measured by span before and after (`tmp/cold2.sh`, spark2), and recorded here.
