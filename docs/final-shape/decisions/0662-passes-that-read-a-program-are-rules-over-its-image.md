# 0662. Passes that read a program are rules over its image; rewrites are edits those rules derive

Status: proposed (2026-11-23). Feeds decision 0639 §6a (the tail's steps by place, kind sets as rules).

## Intro

A program goes through 86 passes between its lowering and its plan. Each pass is a Rust type that implements
`ProgramPass` (the whole program in, a derived program out) or `RulePass` (one rule in, its replacement out), in
`plan_base/.../program_image/manager.rs`. The question asked was whether a more ergonomic pass language should augment
or replace the Rust passes.

The passes do three kinds of work, and each kind has a different natural home:

- **Analyses** read the program as a graph (which rules read which relations, what is declared, what a fixpoint of
  "live" or "empty" reaches) and keep, drop or declare. These are already Datalog problems written as Rust worklists.
- **Local rewrites** turn one rule into another by a pattern in its body: fold a constant, drop an implied literal,
  make a variable used once `_`. They all write through one small vocabulary of edits (`Subst`: a variable to a term,
  a literal dropped, an atom's variables made `_`).
- **Generators** write new relations and rules from templates: demand (magic sets), the three-valued split, the
  findings, the reach form, locating and pushing joins into chunks.

The proposal: **augment, do not replace.** The program image gets a relational view, so analyses are `.rerail` rules
over it, evaluated by the one engine and maintained by delta. Local rewrites become rules that derive edit facts in
the existing edit vocabulary. Generators stay Rust passes, and every pass declares its place (local, upward,
downward, across) so decision 0639 §6a can split the tail by module from the declarations.

## The passes today

The survey reads each pass's type and its doc comment (86 types implement one of the two traits). 11 rule passes and
7 program passes have no doc comment, and are classed by their name.

| Kind | Count | Passes |
|---|---|---|
| Analysis: keep, drop, declare or refuse by a fixpoint over the program's graph | 16 | `BaseInputs`, `Declared`, `DeclareAxes`, `DropRepeated`, `DropSelfRules`, `DropUnread`, `FactsOnce`, `InputsRead`, `KindSets`, `MergeIdentical`, `Prune`, `Unfillable`, `SubsumeRequests`, `GrammarNames`, `Projected`, `DecidedFactTests` |
| Local rewrite of a rule | 28 | the 21 `RulePass`es (`CatVariables`, `ConcatInverses`, `DropImplied`, `DropKindTests`, `ExistenceAsReads`, `FoldConstants`, `MergeKeyed`, `SpanContainment`, `WildSingletons`, `Normalize`, `Absorb`, `GateRules`, `ImpliedSets`, `Lowering`, `MatcherRelations`, `Memberships`, `Nearest`, `NodeSubjects`, `Rounds`, `TextRelations`, `Twins`), `InOrder` (the program pass that runs a rule pass), and 6 program passes that rewrite rule by rule (`LiteralsOnce`, `Unit`, `UnitColumns`, `BooleanRelations`, `GroundRules`, `CpythonFunctors`) |
| Generator: new relations and rules from templates | 42 | the demand passes (`AggregateDemand`, `KeyDemand`, `ProgramDemand`, `NegatedDemand`, `DemandExports`, `BoundProducts`), the reach and rank forms (`ReachForm`, `UndoInCycles`, `RankForm`, `ClimbsAsAxes`), the chunk passes (`Locate`, `MoveCrossFile`, `PushLocalJoins`, `ProjectExistentials`, `FactorizeExports`, `DeferJoins`, `RequestsFileLocal`), the lowering's tail (`ThreeValued`, `Findings`, `OrderNodes`, `SpanPatterns`, `ScopeGlobs`, `WidenCyclicContexts`, `BuiltinsOut`, `CoreDecls`, `SourcesOnRequest`), and `FactSpaces`, `FunctionKeys`, `JoinComplements`, `InlineFilters`, `InlineNodeTests`, `Hide`, `RetypeHelpers`, `StatedApart`, `SetAnswers`, `SetQualifiers`, `GateFields`, `FactsAndSets`, `DeclareInput`, `ProjectAll`, `RuleOrder`, `DropJoin` (a test's) |

An analysis in Rust rebuilds the program's graph each time it runs. `DropUnread` (`simplify.rs`) is typical: it builds
a `HashMap` of reads, runs a worklist from the kept relations, and drops the rules whose head the worklist did not
reach. As rules over a view of the image it is two rules:

```
live(r) :- img_kept(r).
live(s) :- live(r), img_reads(r, s).
img_drop_rule(q) :- img_rule(q, h), !live(h).
```

The same holds for the E10 spike (decision 0639): `KindSets` is three rules over the program's reads, and they take
out the same sets in all 17 libraries.

## The design

### 1. The program image as relations

A view over a program image (`Version`) states its rules as facts. Nothing is copied: the view reads the image's
arrays in place, as the kept facts' virtual tables read their blocks (decisions 0264, SQL compatibility through kept
virtual tables).

| Relation | Rows |
|---|---|
| `img_rule(r, head)` | each rule, by its id, and its head's relation |
| `img_lit(r, i, what, rel, neg)` | each body literal: an atom, an aggregate's atom, a comparison, a functor equation |
| `img_arg(r, i, j, term)` | each literal's argument `j`: a variable slot, a constant, or a term id |
| `img_head_arg(r, j, term)` | the head's arguments |
| `img_decl(rel, cols)`, `img_facts(rel)`, `img_input(rel)`, `img_output(rel)`, `img_kindset(set)`, `img_kindset_kind(set, kind)` | the image's fields; a kind set by its name, so one of no kinds is a row |
| `img_reads(h, b)`, `img_held(rel)` | derived: a rule of `h` reads `b` (positively or not); the image holds `rel` as a relation |

The names are `img_`, not `rr_`: `rr_` is the lowering's own, for the helpers it makes, and the tests set `rr_` names
aside as helpers. `img_reads`, `img_held` and the strongly connected components are rules in the view's library
(`rr/view.rerail`), not Rust: the checks of decision 0639 §6a (components holding the own module's relations) read
them. The edits below are `img_` too.

### 2. Analyses as rules

An analysis pass is a `.rerail` program over the view that derives some of a fixed set of edits:

| Edit | Applied as |
|---|---|
| `img_drop_rule(r)` | the rule is not kept (`ProgramW::keep`) |
| `img_declare(rel, cols)`, `img_undeclare(rel)`, `img_set_input(rel)`, `img_drop_facts(rel)` | the header's fields |
| `img_refuse(message)` | the pass fails with the message (`Declared`, `GrammarNames`) |
| `img_merge(rel, into)` | `MergeIdentical`'s renaming |

The Pipeline applies them as it applies a Rust pass's writes, so a rules pass and a Rust pass are steps of one list,
with the same key, version and `again`. Their evaluation is the engine's, so the incremental case is the engine's
delta maintenance (decision 0225 §5): when a program's own module changes, only the edit facts that depend on the
change are derived again. A rules pass needs no hand-written `again`. This is what decision 0639 §6a needs for the
downward steps: `KindSets` over the union, maintained per program.

There is one evaluator. The prototype's record is plain that two lowerings of one thing need a routing predicate, and
that the routing selected the second engine for nothing (S003, sql-index-prototype design index topic 6a). Phase 0
already plans modules in `bootstrap.rerail`. Analyses over a program are the same language over a different view, not
a second engine (decision 0083's phases).

### 3. Local rewrites as edits rules derive

A local rewrite is an analysis whose edits are the rule passes' edit vocabulary, the operations `Subst` already
carries (`simplify.rs`):

| Edit | `Subst`'s field |
|---|---|
| `img_subst(r, var, term)` | `to` |
| `img_drop_lit(r, i)` | `drop` |
| `img_wild(r, i, var)` | `wild_atoms`, `wild` |
| `img_add_lit(r, template, args)` | a literal from a named template, for the rewrites that add one (`ConcatInverses`, `SpanContainment`) |

`WildSingletons`, `FoldConstants` and `DropImplied` become:

```
// A variable of a positive atom named nowhere else is `_`.
img_wild(r, i, v) :- img_arg(r, i, _, v), img_var(v), img_lit(r, i, "atom", _, false), 1 = count : { img_uses(r, v, _) }.
// A variable equal to a constant is the constant.
img_subst(r, v, c) :- img_lit(r, i, "eq", _, false), img_arg(r, i, 0, v), img_var(v), img_arg(r, i, 1, c), img_const(c).
img_drop_lit(r, i) :- img_subst(r, v, _), img_lit(r, i, "eq", _, false), img_arg(r, i, 0, v).
// A literal equal to an earlier one drops.
img_drop_lit(r, j) :- img_same_lit(r, i, j), i < j.
```

A rewrite pass's rules see the body as a set of literals, which is what a body is. Order-sensitive matching, as in a
term-rewriting language, would have to try each permutation.

Two rewrites of one rule in one round must not clash. A rule is rewritten once a round, from the edits derived on the
round's input. A variable that two `img_subst` rows send to two terms is `img_refuse`d (a rewrite conflict, like 0639's
`helper_conflict`). The Pipeline runs a rewrite pass to its fixpoint as `InOrder` runs a rule pass today.

### 4. Generators stay Rust, with declared places

A generator makes relation names, column layouts and rules from analyses that are themselves programs: magic sets'
adornments, the three-valued split, the chunk passes' location columns. Writing them as templates in the rule
language would need quotation of rules inside rules, and that is a second language with its own typing. They stay
`ProgramPass`es. What changes for them:

- **Each pass declares its place** (decision 0639 §6a): `const PLACE: Place`, one of `Local` (a rule or relation,
  with declarations), `Upward` (a scope's results into a module), `Downward` (from what the program reads) or
  `Across` (relations two modules give rules to). The tail's split by module reads the declarations, and `rr::DOWNWARD`
  becomes the tail's steps that declare `Downward`.
- **Each pass reads its analyses from the view** where an analysis is one of §2's: `AggregateDemand`'s readers of a
  helper and `SourcesOnRequest`'s reads become rules, so the generator's Rust is the writing alone.

### 5. What moves first

1. The view (§1) and the edit application (§2) in `plan_base`, with `KindSets` as the first rules pass (0639's T10).
2. `DropUnread`, `InputsRead`, `BaseInputs`, `Prune`, `Unfillable`: the reachability analyses, each against its Rust
   pass on every pack before the Rust pass is removed.
3. The rewrites with no added literal (`WildSingletons`, `FoldConstants`, `DropImplied`, `DropKindTests`,
   `LiteralsOnce`), then those that add one.
4. `PLACE` on every pass.

#### 5.1 How the first rules pass runs

**The view is the engine's input, read in place.** Each `img_` relation of §1 is a `vtab::Source` over the
`Version` (`program_image::view`), which the native engine binds as it binds any input. A row is an id: a rule's
id for `img_rule`, a rule's id and a literal's number for `img_lit`, an atom for `img_decl`, `img_facts` and
`img_kindset`. Each value is read off the image when the engine asks for it: a name is the image's own atom, a rule
its id. Nothing of the image is copied or interned again.

**The pass's program is lowered once.** A rules pass's program is a library file and its includes (`rr/view.rerail`),
read as one program without `core.rerail`, and lowered once a process: it is a constant of the engine's files. Its
constants (`"atom"`, `"agg"`) are numbered by the program's own interner from 2³¹ up, so none is an image's atom.
Its lowering runs the tail, whose `KindSets` finds no kind set in it and evaluates nothing, so no pass is evaluated
by the program it lowers.

**A run is the evaluation alone.** The thread's connection binds the view's sources by name and runs the lowered
program with the native engine (`run_atoms_native_over`), the one evaluator of S003: no corpus run, no documents.
The outputs stay in the engine (`kept`) and are read where they lie; each name is the image's atom, read back as its
text.

**`KindSets` splits by what each part is.** The membership of each set (`kinds_of`) and the axes over derived sets
are generators (§4) and stay Rust. Which sets are taken out is the analysis `rr/kind-sets.rerail` states, and it
replaces the pass's Rust fixed point. The pass undeclares each `img_undeclare` and drops each `img_drop_facts`.

`img_rule` and `img_lit` number their rules (`r: number`), and `img_decl` gives its relation's arity, which the
image holds, not its columns' text. The view's relations are the program's `.input`s.

**A column the view counts by value prices its probes.** Each indexed column hands the planner its counts
(`JoinCounts::of_counts`, decision 0488), so a probe is priced by the rows it joins. Without them, the planner's
skew-weighted fan started `img_reads_set`'s join from the literals: on python's library it read 8,625 literals for 9
rows. It now starts from the 30 kind sets.

**The library's run is kept with the library, and answers a program it holds over.** A program on the module path
is linked as a union on its library's union (`Version::union_on`, which reuses the library's parts, size and
digest). The steps before `KindSets` write edits, not derivations (`CoreDecls`, `HostCopiesOut`, `Passes`: a rule
the renaming leaves as it is stays in place), so `Version::extends` walks the program back to the library's union.
`RulesProgram::run_on` runs a rules pass over that union once. It runs through a frame (`view::Frame`, 0659's
frames at probe granularity): each probe the engine makes (relation, column, key) is recorded with a digest of the
rows it found, and the frame notes any row a scan read that no probe handed out. The run is kept as an analysis of
the union (`view::Kept`), keyed by the pass's program, the kind sets it was given and its outputs. For a program, the
frame's probes are read again through the program's own indexes. If every digest matches and nothing was scanned,
the kept outputs are the program's, and the program runs nothing. Otherwise the program runs over its own view. A
program that declares or states facts of a kind set the library reads misses by design (node's `n-callback-return`,
a phases program).

Measured on spark2 (pinned to a big core, 100 runs each; spikes P2 and P3), the step costs per program:

| Pack | The step run over the program | Answered by the library's run | Of which the frame read |
|---|---|---|---|
| python (765 rules) | 3,843 µs | 207 µs | 8 µs |
| react (539–923 rules) | 2,357 µs | 175–234 µs | 7–11 µs |
| sql (53–63 rules) | 1,110 µs | 87 µs | 0.8 µs |

What is left is mostly the library's kind sets, made again by every program. The union carries no kind sets of its
own, so each program finds the kinds of the library's sets (`kinds_of`: python 50 µs, sql 4 µs), writes the whole
`Kindsets` field (40 µs and 15 µs), and copies the staged fields onto its edit (`state_fields_of`: 87 µs and
50 µs). Taking the sets out (2.4 µs and 0.4 µs) and the frame read are small. The next lever is kept maintenance:
the library's kind-set map kept with its union, so that a program finds and writes only the sets its own module
adds.

## Prior art

- **ISLE** (Cranelift's instruction-selection DSL, vendored at `native/vendor/cranelift-codegen-0.125.4/src/*.isle`,
  42,657 lines, read first-hand). A rule is `(rule (simplify <pattern>) <replacement>)` over typed terms, with
  extractors and constructors bound to Rust, and priorities to choose between overlapping rules. It compiles to Rust
  matching code. It fits local rewrites of trees whose argument positions are fixed (`(iadd ty x (iconst_u ty 0))`).
  A rule body is a set of literals joined by shared variables, not a tree: an ISLE pattern would name literal
  positions, and its matching would be the permutation search §3 avoids. It has no fixpoint over a graph, so the
  analyses would stay Rust. It would be a second language beside `.rerail` (S003).
- **egglog** (measured in this project, `docs/final-shape/notes/t9-typing-support.md`, the DE runs). Equality
  saturation over terms with Datalog rules: the model of rewrites as facts that a fixpoint derives is §3's. Measured:
  60 ms a document, 2.29 GB for the corpus, and additions only. A file added and re-saturated cost 31% of a full
  evaluation for 0.4% of the nodes, with no retraction. Drop passes are retractions, and per-program maintenance
  (0639 §6a) is a delta, so it does not fit as the engine. The idea it shares, rewrites as derived facts, is kept.
- **Datalog over a program's facts** (Soufflé's and Doop's use; this project's own `bootstrap.rerail` plan). The
  language `.rerail` is Soufflé's dialect. Phase 0 already plans and keys modules as rules over files' facts
  (decision 0639 §1–3). §1–2 apply the same thing to the program image.
- Nanopass (typed intermediate languages per pass), MLIR's PDLL and Stratego's rewriting strategies were not read
  first-hand for this decision and are not cited.

## Collisions

- **A rules pass's cost on a small program.** A Rust analysis on a 200-rule program is a few hash maps. A rules pass
  is a planned program. The view is read in place and the pass's plan is kept with its image (a planned analysis is a
  program like the others, decision 0281's keyed steps), so a rerun is the evaluation alone. T16 measures the kind
  sets as rules against `KindSets` per program, on the packs of 0639's E15.
- **Edits in one round that interact.** §3's rule: one rewrite a rule a round, conflicts refused.
- **Rules passes in the bootstrap.** The passes that lower `bootstrap.rerail` itself cannot be rules evaluated by the
  program they lower. Phase 0's own programs keep the Rust passes until the view's library is lowered once and kept
  in the template (decision 0639 §5).

## Tests, written before the code

- **T15.** The view (§1) states every rule, literal and argument of an image: each pack program's image printed from
  the view's rows is the image printed.
- **T16.** `KindSets` as rules (0639's T10), through the Pipeline: every pack program plans as with the Rust pass,
  and the kind set rules' time per program is reported beside the pass's.

  **Built (2026-11-24):** §5.1. T16 passes on every pack's library tailed to just before `KindSets`: the rules take
  out what the Rust fixed point took out, which the test keeps as its reference; their times per run are in decision
  0639's step 6. T15 passes for the relations §5.1 states (`img_rule`, the literals that name a relation, `img_decl`,
  `img_facts`, `img_kindset`), each compared with the image's own reads. The arguments (`img_arg`, `img_head_arg`)
  and the literals that name no relation are not yet in the view, so an image is not yet printed from its rows.
- **T16b.** On the module path, `KindSets` takes out what the fixed point took out, whether the library's kept run
  answers the program or the program runs, and every program reaches `KindSets` still a union on its library's.

  **Built (2026-11-24):** it passes on every pack, up to four programs each. The kept run answered every program of
  complexity, javascript, next, python, python-main, react, sql and vue, three of node's and typescript's four, and
  one of phases'. blade, php, rerail and rust hold no kind set as a relation, and run no rules.
- **T17.** Each analysis moved (§5 item 2) gives its Rust pass's edits on every pack program.
- **T18.** Each rewrite moved (§5 item 3) gives its Rust pass's rule on every pack program, and a clash of two
  `img_subst` rows is refused.
- **T19.** Every pass declares `PLACE`, and the tail's steps that declare `Downward` are `rr::DOWNWARD`'s.
