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

What stays Rust, and why, is decided by the spike: the walk over a module's clauses, the link, and the lowering's
tail. Each of the three needs a rule run per program to be rules, and a rule run per program costs more than the
lowering it would replace (§ Spike, E5).

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

### 4. The link

For a modular text, `Text::lower_version(true)` becomes:
1. `check_names` and `closed_program`, as now.
2. The text's modules from its graph's plan rows, by rank, each read from its image by its `p0_module_lowered` digest.
3. The text's own module: `lower_module_in_written(own, &modules)`.
4. `link_library_versions(modules ++ [own])`, then `contexts::passes`.

`link_versions` takes the modules by reference and clones only the first module's lowerer. `lower_library` (packs)
takes its modules the same way, and `library_modules_with_files`' graph code (reach, groups, order, `module_key`)
goes. A plain text keeps `lower_statements_version`, and `bootstrap.rerail` itself, lowered before phase 0 runs,
keeps the whole lowering.

**Why the link's decisions stay Rust here.** The link renumbers helpers (`made`: an offset that sums the `next` of the
modules before it), renames Python candidate natives that clash, and merges helpers equal to an earlier module's
(`once_key`: stem, column types and the helper's rules as templates, read through the renames so far). These are
decisions per text, over the text's set of modules. As rules, they would need a rule run per text, which the composed
path exists to avoid. A run of the plan's rules alone took 50–90 ms (E5), against the 25–30 ms the module path saves
a program. The link is not left with decisions instead: making helper names content-addressed (named by their
rules' digest, a self-reading helper by its module too) leaves the link a union with nothing to decide. That is a
change to the lowering's naming, and so to every program's printed form, and is its own decision.

**Why the tail stays Rust.** The tail's whole-program questions (which core relations are read, which kind sets are
read) have no per-text rule run at lowering time to be answered in. They belong to preparation's fact spaces (0246
§3, 0225 §5).

### 5. The template holds every module

`test_store::fill` already gives phase 0 every library file and phase program as seeds, in one run. With §3, that run
asks for every module, so the template's store holds every module's image, and every test's copy of it finds them.

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
| E5 | A run of the plan's rules | 50–90 ms each |

The 7 inline failures fail the same without this change: `ruff-perf403` (refused at reading, open items),
`eslint-curly`, `eslint-no-object-constructor` and `clippy-redundant-field-names` ("message: no column"),
`eslint-one-var` (`ordered` on a `.decl`), and `next-no-page-custom-font` and `eslint-plugin-vue-no-child-content`
(`match_all` over an unbound node).

What the spike leaves: the link, tail and passes are now most of a program's lowering (33 of 39 ms for Python). The tail runs
over the whole library again for every program, so moving its questions per module (0246 §3) is the next lever.

## Tests, written before the code

- **T1.** E1 as a test: for every pack program and every phase program, the module path prints as the whole lowering
  of the same canonical list.
- **T2.** The corpus suite and the rr tests pass, and their outputs are today's.
- **T3.** E5 as a test: phase 0's plan equals the plan over the library's files, and its keys do not change with the
  facts' order.
- **T4.** A cold run of a Python program on a copy of the template asks for no module lowering (`p0_lower_ask`
  answered from kept results), and a store with one image dropped asks for that module alone.
- **T5.** A reused bootstrap run whose module image is gone asks for the lowering again (§3, reuse).

## Steps

1. §1 in `bootstrap.rerail` and the plan rows in `Graph` and its image (§2). T3.
2. §3: the engine answer, its reuse check, and `Graph::holds` over images. T4, T5.
3. §4: single programs onto the module path, `link_versions` by reference, `lower_library` onto the plan rows, and the
   Rust plan removed. T1, T2.
4. §5 checked by T4 on the template, and the template's fill time measured.
5. The cold run measured by span before and after (`tmp/cold2.sh`, spark2), and recorded here.
