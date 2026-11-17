# 0281: Program passes run under a pass manager: rule rewriters and program edits over cached, maintained analyses

Status: accepted (2026-11-15); steps 1–4 built (step 4, the run reads versions, done 2026-11-16); step 5 under way:
(a) and (b) done, step 5 (c) storage done (2026-11-16; §6's keyed steps), its use on the preparation path and the
ep2 measurement after the remaining passes are ported (see Path and Alignment).
Builds on 0173 §9 (a pass is an edit or a derivation, and a derived rule records its sources), 0225 §5 and 0246 §3
(the whole-program questions are rules over the image), 0266 (blocks: a writer, a merged view, base + delta), 0269
(a step's decisions are rows of what it made), 0275 (a step is rederived until its key hits) and 0276 (one form from
the tape on, written through `ProgramW`).

## Intro

0276 moves about 10,000 lines of tree passes onto versions. Moved one by one as they are, each pass would still do
everything itself. Here is what one pass does today, from start to end:

1. It opens a profile span and a decision collector.
2. It computes the analyses it reads: strata, heads, readers, reachability, congruence and domains. It gets them
   from scratch, or from the tree, even when the pass before it just computed the same ones.
3. It opens a builder and decides on its own whether it makes a view, an overlay or a derivation.
4. It walks every rule, copying the ones it keeps one by one, and writes the ones it changes.
5. It sets the inputs, outputs, declarations and `.about` rows its edit needs.
6. It finishes. On failure it falls back to the tree (`made_from`), then notes a version trace.

Only step 4's changed rules, and part of step 5, are the pass itself. The survey counts the rest. In this tree,
`components_of` is called from 12 places, `Fields::of` from 20, `About::of` from 8, `made_from` from 14 and
`Builder::on` from 28. 0246 found congruence closure run five times per load, and `prune` three or four times.

A compiler's passes are not written like that. In LLVM's or MLIR's pass manager, a pass says what it rewrites and
asks for analyses by type. The manager owns everything else:

- the order of the passes;
- caching each analysis and invalidating it when an edit touches what it read;
- the walk over the IR;
- timing, statistics and verification.

This decision gives rerail's program passes that shape. It is designed for where the sibling branches are going:

- **Writers.** Every structure is made by a writer and frozen (0240, 0266 §8).
- **Deltas.** An edit is a delta over a full block, read with its base as one view (0266 §6, 0280, 0259).
- **Linear structures.** Each block is one slab of preorder node arrays and dense id-indexed columns, never trees
  of boxes (0260, 0266). The program image already is one: rules in preorder, CSR indexes, ids global to a stack.

## What the passes are

0173 §9 sorts every surveyed pass into five operations:

1. drop rules or literals;
2. append rules, literals or declarations;
3. rewrite terms;
4. rename relations;
5. move columns.

It also sorts them into two kinds: passes that are local to a rule, and passes that are not. The front and
preparation passes of 0276 §5 fall into three shapes:

| Shape | What it decides | Passes |
|---|---|---|
| **Rule rewriter** | for one rule, read alone: keep it, drop it, rewrite it, or replace it with rules | normalize, physical, `cpython_functors`, `matchers::absorb`, `strings::as_relations`, `node_subject_tests`, `long_text_twins`, `plan::lower`, `qualify`, supports, the per-clause half of `rr::lower` |
| **Program edit** | from analyses of the whole program: a set of rules, relations or fields to change, then a rule rewriter over the rules that set selects | `prune`, `simplify` (by congruence), the merges, `reach_form`, `demand_exports`, `factorize_exports`, `locate`, `push_local_joins`, the gate and split, the cut |
| **Program writer** | a new program from analyses: magic rules, helpers, round programs | `magic` (both demands), `RoundProgram::of`, the cone's parts, the maintained form (0241) |

Every pass is one of these, or a program edit that ends in a rule rewriter. So the API has three pass traits, one
driver that runs them all, and one store of analyses.

## Decision

### 1. A pass says what it changes, and nothing else

```rust
/// A pass local to a rule (0173 §9): what it does to one rule, read alone.
pub trait RulePass {
    const NAME: &'static str;
    /// The rules it visits (§3): all of them, or only those the index finds (heads in a set, readers of a set).
    fn select(&self, cx: &Cx<'_>) -> Select { Select::All }
    /// What becomes of `r`. Writing goes through `out`: a rewrite through a `Rewrite` (build.rs's hooks), or new
    /// rules through `out.rule()`, each recording `r` as its source (`R::From`).
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_>) -> Result<Step>;
    /// Declarations and fields the pass adds once, after its rules (a helper's declaration, say).
    fn finish(&mut self, _cx: &Cx<'_>, _w: &mut FieldsW<'_>) -> Result<()> { Ok(()) }
}

pub enum Step { Keep, Drop, Written }

/// A pass over the whole program: it reads analyses and states an edit.
pub trait ProgramPass {
    const NAME: &'static str;
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()>;
}
```

The three shapes map onto these traits:

- **A rule rewriter** is a `RulePass`.
- **A program edit** is a `ProgramPass` whose `run` reads its analyses, then calls `w.keep(..)` or
  `w.apply(&mut rule_pass)` over the rules it selected.
- **A program writer** is a `ProgramPass` over `ProgramW::derive`, writing term by term.

None of them opens a builder, walks the rules it keeps, opens a span, notes a trace or falls back. The driver
does all of that.

Each example below is the whole pass.

```rust
/// `merge_identical`: each relation congruent to another reads as its class's first; the others' rules go.
struct Merge;
impl ProgramPass for Merge {
    const NAME: &'static str = "merge identical relations";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let classes = cx.get::<Congruence>()?;             // cached, or computed once for this version
        let gone: RelSet = classes.non_leaders();          // dense bitset over the stack's relation ids
        w.keep(|r| !gone.has(r.head().rel()));             // a view: no rule copied
        w.apply(&mut Rename { to: classes.leader_map() }, Select::Readers(&gone))?;
        cx.decide("merged", || format!("{} relations into {}", gone.len(), classes.len()));
        Ok(())
    }
}

/// `Rename` is a `RulePass` whose rewrite is one `atom_as` hook.
struct Rename<'a> { to: &'a RelMap<RelId> }
impl RulePass for Rename<'_> {
    const NAME: &'static str = "rename";
    fn rule(&mut self, _: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_>) -> Result<Step> {
        out.rewrite(r, &mut AtomAs(|a| self.to.get(a.rel())))
    }
}
```

### 2. Analyses are asked for by type, cached by version, and kept up to date from the edit

```rust
pub trait Analysis: Sized + Send + Sync + 'static {
    const NAME: &'static str;
    /// What it reads, so the manager knows which edits leave it standing (§4).
    const READS: Reads;                                    // RULES | DECLS | INPUTS | OUTPUTS | ABOUT, by bit
    fn compute(cx: &Cx<'_>) -> Result<Self>;
    /// It, for the edit's output, from its value on the input and the edit's delta: `None` recomputes.
    fn maintain(&self, _cx: &Cx<'_>, _delta: &Delta<'_>) -> Option<Self> { None }
}

let strata: &Strata = cx.get::<Strata>()?;
```

- **Held by the version.** An analysis is held by the version it is of (`Version::analyses`), beside its indexes,
  and dropped with it (as built in step 2; first drafted as a pipeline's). It is not a process map, so 0103's rule
  (no static memo maps, caches are rules) holds: what outlives a run is a fact, kept by the steps' keys (§6).
- **Invalidated by what the edit touched, not by what the pass promises.** LLVM asks each pass to say what it
  preserves, and a pass that says it wrongly makes a silent bug. Here the edit is a delta the writer recorded:
  which rules were removed and added, which declarations changed, and whether the inputs, outputs or `.about` rows
  changed. An analysis whose `READS` the delta misses carries over unchanged. One it touches is maintained by
  `maintain` if that is given, else recomputed. A debug build recomputes every carried or maintained analysis and
  compares the two.
- **Ids, not names.** An analysis holds dense columns over the stack's ids: `RelMap<T>`, `RuleMap<T>`, `RelSet`
  and CSR groups. Ids are global to a stack (0173), so an analysis of a version reads the same after an edit, and
  the delta maps to it by id. Names appear only where a pass prints. Today's `BTreeMap<String, …>` and
  `BTreeSet<String>` analyses (`components_of`, `needed`, `same_doc`, the split's sets) become columns.
- **The first analyses:**
  - `Fields`, which exists;
  - `Heads` and `Readers`, from the image's CSR indexes;
  - `Strata`, `Components` and `Edges`, from `analyses.rs`;
  - `Needed`, reachability from the outputs, which `prune` reads;
  - `Congruence`;
  - `Domains`;
  - `Constants`, `Locations` and `FileLocal`, the split (0225 §5);
  - `Hiding`;
  - `Reaches`, the closures (0119).

  Each analysis starts as the Rust function it is today, moved behind the trait.
- **The trait is the seam to analyses as rules.** 0246 §3 and 0225 §5 make each of these a rule program over the
  image's relations, maintained by the engine's own delta machinery. Then `compute` runs that program and `maintain`
  is its maintenance. No pass changes: passes ask for `Congruence`, never for how it is found.

### 3. The driver walks rules in order and writes runs, not rules

A pass never iterates over the program to copy it. The driver does that once, the same way for every pass:

- **Selection through the indexes.** `Select::All`, `Heads(&RelSet)`, `Readers(&RelSet)`, `Rules(&RuleSet)` or
  `Delta`. A pass that rewrites the readers of three relations visits their rules, found by CSR, and costs those
  rules, not the program.
- **Coalesced runs.** Rules are visited in id order. Each run of rules kept in a row becomes one range: a view's
  range, or one node-range copy in a derivation (0276 §3's `copy`). A dropped rule ends a run. A rewritten rule is
  encoded into the delta's node array in order. So the output is linear: a range list and an appended node tape,
  never a rule-by-rule rebuild.
- **The form is the writer's choice.** The pass does not pick view, overlay or derivation. Removals only make a
  view. Additions and field changes make an overlay. A pass that rewrites most rules makes a derivation, so its
  output keeps `from`. A delta of a delta is folded into one cumulative delta over the full block (0266 §6).
- **Incremental for free where 0173 §9 says it can be.** A `RulePass` reads one rule alone. When its input changes
  by a delta, the driver re-runs it only on the delta's added rules, and drops what came from the removed ones
  through `from`. A pass need not know it is maintained.
- **Parallel by morsel.** A `RulePass` that holds no state across rules (its type says so: `RulePass + Sync`, with
  `rule(&self, …)`) has its selected rules split across the pool. Each morsel writes its own node run, and the
  driver concatenates the runs in id order. The form matches 0266 §8's `writer()`: one writer per morsel, each
  `Send`.

### 4. The delta is the edit's record, and every consumer reads it

```rust
pub struct Delta<'a> {
    pub input: &'a Placed,
    pub output: &'a Placed,
    pub removed: &'a RuleSet,                // the input's rules the output lacks
    pub added: RuleRange,                    // the output's own rules, in order, each with its `from`
    pub decls: &'a RelSet,                   // relations declared, undeclared or changed
    pub touched: Reads,                      // which field kinds changed
}
```

The writer makes the delta as the edit is written. It has three readers:

- **The analysis store** reads it for §2's invalidation.
- **The driver** reads it for §3's incremental runs.
- **0266's `Program` block** stores it as the delta block over its full one.

So one record serves the cache, maintenance and storage, and no consumer diffs two versions to find out what
changed. `Version::diff`, which diffs a tree against a version, goes once the front passes are writers.

### 5. A pipeline runs passes and owns the cross-cutting work

```rust
let prepared: Placed = Pipeline::new("prepare", &settings)
    .run(Prune)
    .run(Simplify)                              // a ProgramPass reading Congruence
    .run(Prune)
    .run(ReachForm)
    .run_if(demand_on, DemandNegated)
    .run(Absorb::new(capture))                  // a RulePass
    .run(StringsAsRelations)
    .run(PlanLower::new(dialect))
    .until_unchanged(|p| p.run(Merge).run(Prune))
    .finish(lowered)?;
```

For each pass, the pipeline does what each pass now does itself:

- **Profile span:** one per pass, named by `NAME`.
- **Decisions:** collected per pass (0269's `Deciding`), as rows of the step's output.
- **Version trace:** the input, output and delta's sizes per pass, and the form the writer chose.
- **Cost:** the delta's size against the input's, feeding 0266's cost rule for roll-up.
- **Fixpoints:** `until_unchanged`, where `prune` and the merges now loop by hand.
- **The tree check:** a pass not yet moved runs as `TreePass(fn(front::Program) -> front::Program)`. A moved pass
  is checked against it by digest of the printed rules, as 0246 §1 does for lowering's forms. When the tree form
  goes, `TreePass` goes with it.

A failing pass fails the pipeline, with the pass's name. There is no fallback to a tree.

### 6. Steps are keyed, so a pipeline is rederived until a key hits

Each pass's step has a key: the input version's digest, the pass's `NAME` and version, and the settings it reads.
This is 0275's chain carried to every pass. A pipeline asked for its output looks for the latest step whose key
the store holds, and runs only the passes after it. What the store keeps of a step is its output image and its
decisions (0269), as facts of the prepared program. No cache store, directive or memo map is added: this is 0103's
two-rule form, where the key rule is the step's key and the output rule is its stored image.

### 7. Writing stays `ProgramW`'s

The writer of 0276 §3 is unchanged. A `RulePass` writes through `RuleOut`, a view of the writer bound to its rule:

| `RuleOut` call | What it does |
|---|---|
| `rewrite` | the rule through a `Rewrite` |
| `rule()` | a rule written term by term |
| `keep_literals(pred)` | the rule with some literals dropped |

Each records `from`. A `ProgramPass` writes through the whole `ProgramW`, and its `fields()` sub-writer covers the
declarations and `.about` rows. The pass traits sit above the writer. Neither knows the other's internals, and 0266
step 3 replaces what is under the writer.

## What it costs, what it saves

- **The framework** is about 1,500 lines:
  - the three traits;
  - the driver's selection, coalescing and morsels;
  - the analysis store with its invalidation;
  - `Delta`;
  - `Pipeline`;
  - the dense `RelMap`, `RuleMap` and `RelSet` columns.

  `Builder`, `Rewrite`, `RuleBuilder`, `Placed`, `Fields` and the CSR indexes already exist.
- **Per pass,** what goes is the builder setup, the copy loop, the span, the trace and the fallback, and every
  analysis computed inline. In supports (2532b2a90), that is its first 25 lines, the copy arm of its loop, and its
  finish and fallback.
- **At run time:**
  - analyses are computed once per version, not once per pass;
  - a rule pass costs its selected rules;
  - a maintained pass costs its delta;
  - a warm run with stored steps runs no pass.

## Path

1. **The framework,** with the passes that already make versions ported to it: `normalize`, `stated_apart`,
   supports, `answers::gate`, the split, `RoundProgram::of`, `qualify` and the cut. Check: their tests, plus a
   driver test that a pass costs its selection.
   - Done: `program_image::manager` (the traits, `Select`, the driver, `Delta`, the analysis store, `Pipeline`, the
     dense `IdSet`/`IdMap` columns); `About`, `Strata`, `Needed` and `FieldsData` as analyses. Rule passes:
     `Normalize`, `Supports`, `GateRules` (the gate's rewrite) and `Rounds` (`RoundProgram::of`). Program passes:
     `StatedApart`, `GateFields`, and the parts' `Retain`, `PartOf`, `SetAnswers` and `SetQualifiers`, which
     `Version::part`, the split's `Loading::parts` and the cross part's `qualify` run. Tests:
     `a_rule_pass_costs_the_rules_it_selects`, `analyses_carry_over_edits_that_miss_what_they_read`,
     `a_pass_that_writes_for_a_kept_rule_fails`, and the passes' own.
   - As built: `RulePass::finish` takes the `Builder` itself (no separate `FieldsW`); `ProgramW` has three forms, an
     edit (`on`), an overlay never folded (`overlay`, a part's fields) and a derivation (`derive`); `Delta` is read
     off the version the writer made (an overlay is its delta, a view its ranges; fields by their hashes); a
     derivation that changes nothing, and an edit that states nothing, return their input, so its analyses carry. A
     rule pass selecting all rules runs as a derivation (the program's order kept); one selecting fewer, as an edit.
   - The cut (`retain::cut`) writes no version: it is an analysis (which strata a run skips, given its inputs'
     digests), and moves with step 2.
   - Not yet: morsels (§3); re-running a rule pass on a delta alone (§3), which needs a stored step to re-run from
     (step 6).
2. **The analyses** behind the trait: `Fields`, `Heads`, `Readers`, `Strata`, `Components`, `Needed`, `Domains`, and
   the split's columns. Each is checked by the debug build's recompute-and-compare.
   - Done: analyses are held by the version (`Version::analyses`, beside its CSR indexes, dropped with it), not by
     one pipeline: every `Placed` the run makes of a version again (`Lowered::placed`) reads one computation. A
     pipeline carries them from a pass's input to its output across the delta. `Components` is an analysis, and
     `Placed::components()` (21 readers) returns the version's; `Placed`'s fields come from the version's
     `FieldsData`; the run's `About::of` reads go through the store; the split is the `Split` analysis; `supports`
     reads `Components`. Test: `every_placed_of_a_version_shares_its_analyses`.
   - `Heads` and `Readers` need no analysis: they are the image's own CSR indexes, which an overlay's delta carries.
   - The cut reads the components, but depends on a run's inputs, so it is not a version's analysis.
   - Left for the steps that move their passes: `Domains` and the 11 `front::components_of` readers, which read
     the tree (lowering, `magic`, `simplify`, `emit`: 0276 steps 4 and 5); the retired rounds and spike (0276 §9).
3. **Rerunning on a delta (§3, Q18).** A pipeline kept with its last output (the passes' stages and states), run
   again on an input that is a delta of its last input: each rule pass visits the delta's added rules alone and
   drops what came from its removed ones (through `from`); a program pass reruns from its maintained analyses.
   Measured on ep2's edit (Q18).
   - Done: `Pipeline::keeping` and `done` keep a run (`Run`: each step's input and output, and for a rule pass its
     selection, the analyses it asked for, and which of the output's rules are the input's own);
     `Pipeline::again(run, v)` runs the same passes over `v`, an edit of the run's input. Per step, against the last
     run's step at its place (matched by the pass's name):
     - its input unchanged: a local rule pass's output is reused;
     - its input an edit of the last (`Delta::of`, not a new base): the last input's analyses are carried to the new
       input across the edit (§2), so a program pass, which runs whole, reads them at the edit's cost. A rule pass
       reruns on the edit alone when it is local (below), its `begin` selects the same rules, and each analysis it
       asked for is the same on the new input: it visits the added rules it selects, copies the rest, and drops what
       came from the removed ones through the version's `from` index (`Version::derived`), or the removed rules
       themselves where its last output held them as they were. The output is an edit of its last output, so the next
       step's input is again an edit of its last;
     - otherwise it runs whole.
   - A rule pass is local when it says which changes make it run whole (`RulePass::WHOLE`, `Reads::ALL` by default):
     it reads its rule and its `begin`'s analyses alone, keeps no state across rules, and returns its results in its
     output. Each stage says how it ran (`Ran`: whole, on the delta, reused).
   - Tests: `a_rule_pass_run_again_visits_the_edits_rules_alone` (two edits in a row, both as an edit of the input and
     as a derivation, each against a run from scratch by digest, then a reuse) and
     `a_rerun_runs_a_step_whole_when_the_edit_reaches_past_its_rules` (a program pass, a default rule pass, and a
     local one whose `Needed` the edit changed).
   - Not measured on ep2 yet: no pass on Q18's path (`merge the programs`, `prepare`) is on the manager, and the
     passes that are (supports, the gate) read the whole program in `begin` (their own component graphs) and number
     their helpers across rules, so they keep `WHOLE = ALL`. Q18's measurement moves with step 4, whose passes are
     the first written local; a session keeps its `Run` beside its program then.
4. **The run reads versions** (0276 step 2; the order chosen 2026-11-16, below). Every reader after `plan::lower`
   reads the prepared program's version through `Placed` and `Fields`, so `Lowered`'s tree goes and the program
   crosses from tree to version at one encode, never back.
   - Done (slice 1): the prepared program's one image, made after the last pass that edits its tree and held beside
     it (`Tree::beside`); the split's parts are views of it (`file_local::programs_over`); a program read from the
     store is held beside its stored image. Read from the version: the declarations' widths and types (20 sites,
     `Fields`), the relations read, the constants tested, `Own::constants`, the statistics (`Stats::of_schema` made a
     second image of the program; `Stats::empty_of`), the cone digests, the documents check (`Needed`), the embed
     slice (a part of the version, `Version::part`), `decode_kept`; `remember` stores the version's image bytes
     (`program_text::store_version`) where it encoded the tree again. The guard
     `a_maintained_run_decodes_and_copies_no_tree` now counts the prepared program's tree too, and passes.
   - Done (slices 2–7, counted by making `Tree` not dereference and reading the compiler's errors: 159 sites before
     slice 1, none after slice 7): the split's parts always have versions; the cross memo, the run's parts, the dump
     and the trace, the session's cut, deltas, qualifiers and maintenance, and the diagnostics read versions; the
     tests read versions or digests.
   - Done (slice 8): `Lowered`'s tree is gone. `lower::Lowered` is a version, its maintained parts (`maint`) and what
     the run reads beside them; `lower::Given` is the tree `lower::lower_tree` gives, which the preparation passes
     edit (`transduce_nearest`, then `corpus`'s) and the SQL emitter reads (`emit::emit`), and `Given::image` makes
     the `Lowered` (`lower::lower` is both). `lower::Tree` (given, beside, of, maintained) and its read counters are
     deleted; the split's `Programs` holds no part trees (`assemble` no longer copies the program twice), and a
     prepared program holds no lowered tree. A tree is made only by decoding a version (`Version::program`, counted
     per thread: `program_image::trees_decoded`, which the guard reads). A program the image has no form for is
     refused at `image` (its rules alone had no fields, so did not run either).
   - Still trees, as preparation (step 5): the split's own analysis (`found`, `crossing`, `constant_relations`,
     `cross_causes`), run once on the tree before its image; and the loader, which decodes a stored image to that
     tree for `prepared_of` (step 7).
5. **The preparation passes on the manager:** `locate`, the merges, `demand_exports`, `factorize_exports` and
   `push_local_joins` as passes over the prepared program's version, before its one encode moves earlier. Written to
   run on a delta: helper relations named from their content, not numbered across rules (`fl<k>_`), and the split's
   file-local set an analysis a rerun compares. With them, steps kept (§6, moved up from step 8): each step's output
   image stored under its key, so a new process finds the last run's steps and reruns on the edit. Measured on ep2's
   edit as Q18 was (a new process).
   - The order (2026-11-16): the passes' last reader first, as in step 4. The split reads their output tree after
     the one image, so the image can move before a pass only once the split reads versions: (a) the split; (b) the
     passes from the last backward (the inputs' pruning and the rule order, the merges, `locate`, `demand_exports`,
     `factorize_exports`, `push_local_joins`), each re-splitting through (a)'s analysis maintained across its edit
     where it split the tree again each round; (c) the steps stored, and ep2 measured.
   - Done (a): the split is `file_local::FileLocal`, an analysis of the version (`on_version.rs`): `split_in`'s
     fixpoints, rule forms and causes one for one, on the image's numbers (a relation its `RelId`, a variable its
     `Slot`) where the tree's numbered relations by their names' addresses (`shape::Rels`) and variables by first
     appearance (`shape::Form`, 0225 §4's interim numberings). `maintain` takes the place of `split_memo`'s kept
     splits on the run's thread: the relations whose rules or declaration the edit changed, by content hash
     (`RuleRef::hash`), and those reading them are computed again, the rest kept; an edit of half the rules or more,
     or of what every relation's place reads besides rules (inputs, facts, `.about` rows), splits whole. `assemble`
     and `crossing` read the version; `programs_of(version)` splits a version, and the loader opens the stored
     lowered program's image with no tree decoded (`program_text::load_image`; `load_version` and
     `read_stored_version` removed). While the passes split the tree too, a debug build checks the version's split
     is the tree's on every prepared program (`programs_over`). A cause's words are the first failure a split met, so
     a split maintained from an earlier one can word one differently from a whole split ("reads cross-file r"
     against "reads r with no location"): which relations have a cause is compared, not the words. Test:
     `the_split_of_a_version_is_maintained_across_an_edit`.
   - Done (b), first slice: the image is made after the merges, and the passes after them run on the manager
     (`corpus::preparation`): the inputs the rules read (`InputsRead`) and the measurement aid's rule order
     (`RuleOrder`), the pipeline's output folded into a base for the store. The domains are read from the version
     (`lower::domains_of`, `domains`' fixpoint on handles, in the tree's order), and the split from it
     (`programs_of`); a debug build checks both against the tree's, its inputs pruned the same way. The manager's
     "a derivation that changed nothing is its input" compared digests, a sum blind to the rules' order, so a
     reordering derivation was taken for its input: it now compares the rules in order too. Tests:
     `the_domains_of_a_version_are_the_trees`, `the_passes_after_the_image_prune_the_inputs_and_reorder_the_rules`.
   - Done (b), second slice: the merges (`corpus::merge_and_drop_repeats`) are manager passes,
     `preparation::DropRepeated` and `MergeIdentical`, and the image is made before them. `congruence` is read from
     the version: a rule's template is a token list mirroring `template::canonical`'s text (variables by first
     appearance, relations read a placeholder, `gated` and a count's `over` unwritten as the text leaves them), its
     partition refinement and the classes' naming and writing `merge`'s one for one; a head is renamed through a new
     `Rewrite::head_as`. A debug build checks the merged version's size and digest and the counts against the
     tree's merges. Test: `the_merges_on_a_version_are_the_trees`.
   - Done (b), third slice: `locate` is a manager pass (`file_local::Locate`, `typing/file_local/passes.rs`), the
     image made before it. It reads the split from the analysis (`FileLocal`, `crossing` on the version) and writes a
     derivation in the tree's rule order: a target's rules made as `<r>_at(d, …)` term by term (the document a
     column a rule binds, one it leaves unnamed, or one through `docof` or `node`) and the cross-file read
     `r(…) :- r_at(_, …)` appended. A debug build checks the result against the tree's `locate`. Test:
     `rows_are_located_on_a_version_as_on_the_tree`.
   - Done (b), fourth slice (2026-11-16): `demand_exports` is a manager pass (`file_local::DemandExports`,
     `export_demand/on_version.rs`), the image made before it. Ported exactly first, helpers renamed from their
     content after (the user's choice, 2026-11-16): an exact port can be checked by digest, a renaming cannot. Its
     supersets, keys and cuts are the tree's one for one. Variables are read by name, as the tree reads them, so
     each set and search runs in the tree's order. Each round's cut is built as a version of its own and split
     through the `FileLocal` analysis, where the tree split its edited program again; the round that stays
     file-local is written as the pass's derivation. That costs one extra split per prepared program, until the
     analysis is carried from the candidate to the output. A debug build checks the lines and, through the passes
     after it, the program's digest against the tree's `demand_exports`. On the 317 B-now rules it cuts 483 exports
     through 166 supersets, as the tree does. Found by that run and fixed: the merge port asked an undeclared
     helper's types (`Fields::types` panics where the tree's merge reads none), and a CLI built with
     `--features test-hooks` panicked loading the test-only dialects' ast-grep grammars, which only the
     dev-dependencies enabled (`crates/lang`'s `test-hooks` now enables them). Test:
     `exports_are_cut_on_a_version_as_on_the_tree`.
   - Done (b), fifth slice (2026-11-16): `factorize_exports` (decision 0099) is a manager pass
     (`file_local::FactorizeExports`, `passes/factorize.rs`), the image made before it, off where the run's options
     turn the factorized store off. `relation_keys`, `determined` and `functional` are read from the version by
     name, and the parts from `keeps::factors_of`. The names follow the tree's counters: a part is
     `fz<declarations>_` and a reader's open variable is `fz<copy>_`, and the readers' copies are made on the
     tree's stack, so each copy's names are the tree's. A relation one of whose rules has an untyped part leaves
     the rules and declarations its earlier rules made, as the tree does. Each round's program is a version split
     through `FileLocal`. A debug build checks the lines and the digest against the tree's pass. On the 317 B-now
     rules it factorizes 66 exports, as the tree does. Test: `exports_are_factorized_on_a_version_as_on_the_tree`.
   - Done (b), sixth slice (2026-11-16): `push_local_joins` is a manager pass (`file_local::PushLocalJoins`,
     `passes/push.rs`). With it, every pass after `bound` runs on the manager, and the image is made straight after
     lowering. It follows `push_with`'s steps: the components, the narrowing tests (`independent_node_groups`,
     `unrestricted`, `descends`, the t31 A1 composition filtered from outside) and the interfaces. Helpers are named
     `fl<k>_<head>`, from the tree's counter and kept per (rule, component) from round to round. A rule's level rises
     for each helper the round's split does not find file-local, and a second whole push leaves the unrestricted
     components when the cross part reads base relations anyway. The rule-building parts the factorized store and
     this pass share (`Made`, `BodyItem`, `Round`, `write`) are in `passes.rs`. A debug build checks the helpers
     made and, through the passes after it, the digest against the tree's `push_local_joins`, and on the 317 B-now
     rules they agree. Test: `local_joins_are_pushed_on_a_version_as_on_the_tree`.
   - Done (b), seventh slice (2026-11-16): the helpers are named from their content. A pushed join is
     `fl<h>_<head>`, a factorized part `fz<h>_<rel>` and its key `fzk<h>_<rel>`, where `h` is a stable FNV-1a hash
     of the text of the rule that makes the helper (`file_local::helper_name`). Counters numbered them before: the
     program's declarations for `fz`, and a counter over the rules for `fl`. A reader copy's open variables are
     `fz<depth>_v`, numbered by how many reads that copy has had replaced, where a counter over the program renamed
     every later reader's. The names keep the `fl<digits>_<head>` shape that the run's and the profile's readers
     parse. A salt hashes again a name the program already declares, or one this round gave other content. The tree
     passes name their helpers the same way. So the debug check still compares the version's program with the
     tree's by digest, exactly, not just up to helper names as planned. On the 317 B-now rules they agree. The
     names change once, so every prepared program and file-local part is a miss once. Tests:
     `an_edit_renames_only_its_own_rules_helpers`, and the three ports' tests.
   - (c), measured first (2026-11-16, ep2 on the profiling build: corpus0 × the 317 B-now rules, one store, B101's
     body edited by one always-true literal, a new process, findings unchanged at 427, load 26–48):

     | Step | Instructions | CPU | Wall |
     |---|---|---|---|
     | warm | 6.1 G | 0.35 s | 0.39 s |
     | one rule's body, a new process | 150.2–150.4 G | 9.0–9.2 s | 5.5 s |

     Profiled: `local` 1.9 s, `parse` and `batch` 1.7 s (workers), `prepare` 0.72 s, `combine` 0.61 s, then `wait`,
     `stats` and `cross` 1.0 s. The edit runs every file's file-local part again: its digest keys each file's memo
     (0225 §7, a prepared part per module), and that is most of it. In `prepare`, the passes this step moved are
     0.45 s: push 0.21 s, demand 0.08 s, factorize 0.07 s, locate 0.04 s, merges 0.03 s.
   - What that means for (c): storing the steps buys this edit nothing yet. The edit changes the pipeline's input,
     so every step's input changes, and a program pass runs whole on a changed input (`Pipeline::program_step`
     takes no prior step). Only a rule pass reruns on a delta. A stored step is reused only when its input is
     unchanged, and then the prepared program's own key (0073) already hits.
     - **Lever 1 (0225 §7):** key the file-local part per module, so an edit doesn't run every file again.
     - **Lever 2:** passes that cost their delta. Push's components are per rule given the split, so it can become a
       rule pass. Factorize and demand decide per relation, then check the split once per round.
     - **Done meanwhile:** each round's candidate program is an overlay of the input holding the round's change,
       not a derivation of the whole program (`passes::candidate`), and its split is carried across that edit
       (`manager::carried`). That cut push's own time from 0.19 s to 0.13 s. The splits stay whole: round 1
       rewrites hundreds of rules, past what `maintain` keeps. Chaining each round on the last round's candidate
       would make rounds 2 and later cost their retries.
   - (c), the storage (2026-11-16): §6's keyed steps, built in the manager.
     - **The key.** `Pipeline::keyed(name, v, &steps)` keys each step by a chain: the input's identity
       (`manager::identity`: its digest, size, and its rules' hashes in order, since the digest is blind to order
       and `RuleOrder` changes only order), the pipeline's name and the engine build. Then for each step, its
       pass's `NAME`, `VERSION` and `settings()`. Both pass traits gained `VERSION` (default 1, bumped when what the
       pass makes changes) and `settings` (default none; `FactorizeExports` gives its switch). A step's input is the
       step before's output, so the chain names it as that input's digest would, and every key is known before
       anything runs.
     - **A caller's key part.** `key_part(label, bytes)` adds a value to the key of every later step: what the passes
       read that is not in the input version. 0298's general forms add their `olds_signature` this way, beside the
       program's digest, which the chain's start already names.
     - **Lookup.** A step whose key the store holds is not run. Its stage is `Ran::Stored`, its decisions are read
       back, and its output is opened only when a later step runs, at `finish`, or on `current()`. So a pipeline
       runs only the passes after the latest stored step, and one whose every step is stored opens one image. A
       step that runs stores its output and decisions under its key. A skipped pass has not run, so what it returns
       in itself, beside its output and decisions, is not there; `ran(i)` says which ran. A keyed pipeline is not
       a kept one (`keeping`, `again`): one carries steps across processes by key, the other within a process by
       place.
     - **The store** (`program_text::StoredSteps`, behind the manager's `Steps` trait): a step is facts in 0103's
       two-rule form: `pipeline_step_key(key, pass, version)`, the key with the pass's version as its tag, and
       `pipeline_step(key, image, size, digest)`, the step it names, with `pipeline_step_decision(key, k, pass,
       what, detail)` (0269). They are one source under the key's alias, and the output is a program image kept as
       a prepared program's is (`store_image`, folded to a base first when it is a view or overlay). No cache store,
       directive or memo map. A step whose image the store lost is not found, and runs again.
     - **Test:** `a_new_process_reruns_only_the_passes_after_a_stored_step`: three passes, the store opened again
       for each run. A changed setting on the last pass runs that pass alone, with the first two read back and
       their decisions too. Nothing changed runs none. A caller's key part runs all, then none under the same part.
       Each output is the unkeyed pipeline's, rules in order.
     - **Not yet:** the preparation pipeline is not keyed. Its passes return results in themselves (the pushed
       relations, the demand lines) that the run reads; they move to decisions first. ep2 is measured once the
       remaining passes are ported.
   **The order from here (2026-11-16, user: the per-policy preparation path, then the workplan to its end, no
   step-by-step tuning).** An edit of one policy costs that policy only when the run's input is an edit of the last
   run's input, and every pass from the rule texts to the prepared program runs on versions and reruns on an edit
   (§3) or maintains its analyses (§2). About 16,000 lines of those passes still edit `front::Program` trees, so the
   per-policy path is steps 6 and 7 carried through, then composed:
   - **6a–6e**, the preparation's tree passes on the manager, last first, the image made one pass earlier each time,
     each an exact port checked against its tree form by digest in a debug build:
     - 6a: `bound` and `transduce_nearest` (done, b0f27d065);
     - 6b: `lower_tree` (done, 07db32f7e);
     - R (found 2026-11-16, before 6c): the passes from 6c on edit `prog`, the tree `Prepared` keeps, which about
       40 sites of the run read after preparation (the projection's text demand, the settings, the outputs, the
       world, the references, the cone, the constants). Moved, they would change what those sites read. So
       `Prepared` first keeps the program before lowering as a version (`source`, the image 6b makes), the readers
       read it (each checked against the tree's answer in a debug build), the store keeps it in place of the tree's
       text, and `Prepared::prog` is deleted. From then on the source is a stage of the pipeline;
     - 6c: `strings::as_relations`, `long_text_twins`, `node_subject_tests`, `matchers`;
     - 6d: `reach_form`, `magic`'s two demands, `undo_in_cycles` (done, with 6e);
     - 6e: `prune`, `simplify`, `base_inputs`, `cpython_functors` (done: the image is made from the program as the
       rule texts give it, and every pass from there runs on the version; a debug build runs the tree's passes beside
       them, stage by stage (`preparation::STAGES`), and checks each stage's version by digest, naming the rules,
       facts and rows only one side has; `simplify`'s `similar_relations` is ported too, profiled runs only).
   - **7a**, combining on versions: each policy's lowered image (0246 §6) renamed into the combined version, and
     combining's passes (the requests, hiding's bound, the fact spaces, the merge of identical relations) on the
     manager. In two halves, each checked stage by stage against the tree in a debug build, as 6c–6e are:
     - 7a-1, the combined program a version: made where the parts are linked, before the passes on the whole
       (core's projections no inputs, `retype_helpers`, `sources_on_request`, hiding's bound once, the fact spaces'
       guards, `same_relations`), which run on it; the combine's caches (the process's and the store's `combined`
       entry) keep the version, the store its image in place of the `.rr` text; the prepared path's edits before
       its image (`fold_own` and its merges, `cross_memo::add`, the components' output) run on it, and the
       prepared key reads it. The run holds no combined tree.
       *First half done (2026-11-16):* the run takes the combined program as a version (`combine_version`, kept for
       the process by the texts' digest) and its header. Every reader before preparation asks the header, or the
       rules' shape (`corpus::shape::Shape`: what rules make and read): the bridged programs (over the version
       pruned), the phases, the constants (`foldable_of`, the split's constants by the same fixpoint), the
       repository inputs, the guests' requests (`analyses::needed`), the world and the components. The edits run
       on the version: the bridged facts and phased inputs, the constants' rows and outputs, the inputs' rows, the
       requests' outputs (each a header edit, `Builder::set_header`), `fold_own` and its merges
       (`combining::fold_own_guests`), `cross_memo::add_to`. A debug build makes each edit on the tree beside it and
       checks the two images agree before preparing.
       *Done (2026-11-16):* `combine_read` is split at the link (`combine_linked`, which returns the linked tree and
       what its tail does, `Tail`). `combine_version` makes the linked tree's image and runs the tail on it
       (`tail_version`): `whole::Projected`, `whole::RetypeHelpers`, `on_request::SourcesOnRequest` per round of
       `ON_REQUEST` (`Widen` with it), `hide::Hide` once when the libraries are held once and a program makes
       guests, `combining::FactSpaces` and `combining::same_relations`. A debug build runs the tree's tail
       (`tail_tree`, what the tools keep) beside it and checks the two images agree. The store's `combined` entry
       keeps the version's image as it is (`combined::remember_version`) and opens it with no tree decoded
       (`combined::lookup_version`). The link itself still builds a tree from the parts: 7a-2.
     - 7a-2, the parts as versions: `lower_in` hands on the stored image unread (it decodes it today), the linked
       library's image likewise, and the per-part passes (`own_part`, the requests' units, `undo_on_request`, each
       part's hiding when the libraries are not held once) run on them; the link renames each part's version into
       the combined one (`Builder`), with the kind sets shared by their kinds.
       *Done (2026-11-16):* `combine_linked` is written once over a part (`corpus::link::Part`): a tree, the tools'
       and a debug build's check, or a version (`link::Versioned`), the run's. What it reads of a part is its header
       and its rules' heads and reads; what it edits is the header and which relations' rules are kept. `own_part`,
       the requests' units (keyed by each form's own rule text), `undo_on_request` (over `rr::lower::RuleSet`) and
       each part's hiding (`hide::Hide`) run on either. `Text::lower_in_version` hands on the stored image unread,
       and the library keeps its linked version (`Library::version`), its tree decoded only when a cone is copied
       from it. The link merges the headers (`link::link_header`) and copies each part's rules from its image
       through the renames (`link::Names`, a `Rewrite`), `core.rr`'s repeated rules dropped once linked. A debug
       build links the trees beside and checks the two images agree.
   - **P**, the per-policy edit: the combined version is kept, and a run's is an overlay of the last one's (the
     edited policy's rules out, its new ones in). The preparation pipeline runs `again` from its stored run. Each
     pass reruns on the delta, or maintains what it reads, or runs whole where it must. ep2 is measured once, at the
     end.
   - **7b–7c**: `rr::lower`'s per-clause half as rule passes, the loader writing versions; `front::Program` deleted.
   - Then step 8, then the blocks port.
6. **`plan::lower` and 0276 step 4's front passes** in the pipeline: `prune`, `simplify`, `reach_form`, `magic`,
   the matchers and the strings, each checked against its tree form by digest, the encode moving earlier with each.
7. **The per-clause half of `rr::lower`** as `RulePass`es over the tape's clauses (0276 step 5), with `Domains`; the
   loader writes versions, and `front::Program`, its statements and the encode are deleted.
8. **Analyses as rules** (0225 §5), one at a time behind the trait. Morsels (§3) with the first stateless pass
   measured to want them.

## Alignment (2026-11-15, after merging final-rerail-shape-rr-align)

- **Q18 (work queue) is this decision's measurement.** An edit's whole-program passes, run again from scratch on
  ep2: `merge the programs` 0.47 s (hiding's bound 0.20 s, the fact spaces 0.13 s, merging equal relations
  0.07 s) and `prepare` 0.59 s (pushing file-local joins 0.14 s, prune, simplify and demand 0.10 s, the memo's
  program 0.04 s, nine more of 0.02–0.04 s); unprofiled, `combine` 13.0 G and `prepare` 9.8 G of 82.9 G. Q18 was
  handed here: "the driver reruns passes on a delta only". So rerunning on a delta (§3) comes next, before the
  remaining preparation passes move: step 3 below.
- **0266 §6's roll-up rule replaces the fold at half.** A version rolls up when its delta has cost what a roll-up
  costs (ski rental), not at a fixed fraction. The writer's fold at half its parent (`Builder::finish`) moves to
  that rule when the program image is 0266's `Program` block; the pipeline's per-pass delta sizes (`Stage`) are
  the cost it counts.
- **0266's `Program` block holds modules whole, a delta the replaced ones.** An edit's input delta to the
  pipeline is the rules of the modules it replaced: what step 3 reruns the rule passes on.

- **The blocks port is this branch's, last (final-rerail-shape, 2026-11-16).** That worktree's 0293 states the
  target: 0173's `Version` interface stays, and 0266's blocks go underneath it. The port of the program image (about
  7,000 lines) is handed here, after the layer has been used on `Outputs` and the memo there; meanwhile that branch
  builds what the port needs under the layer (any frozen version usable as a parent). What decides it is the trace
  check on dl1: whether 0266's one delta expresses 0244's views at the cost measured. The manager sits above
  `Version` and `Builder` (its delta is read off the version an edit made, its writes go through the writer), so it
  is unchanged by the port.

- **The order to the end of the tree (the user's choice, 2026-11-16).** Readers first, then back to the loader
  (0276's order): each step removes a consumer of `front::Program`, so the program crosses from tree to version at
  one encode that moves earlier, and is never decoded back. The other orders were weighed: the preparation passes
  first leave the split and the run's readers on trees, so each run would decode the passes' output whole until
  they move; the loader first decodes the whole program for every front pass. Storing the steps is what lets ep2's
  edits, each a new process, rerun on a delta, whatever the order.

## Agreement with prior work

- **0173 §9:** keeps its two kinds of pass and its `from`; the driver turns them into incremental runs.
- **0225 §5, 0246 §3:** analyses become rules behind the `Analysis` trait.
- **0266:** the writer, the merged view and base + delta. `Delta` is what the `Program` block stores, and the
  morsel writers are §8's.
- **0269:** decisions are collected per pass.
- **0275:** a step's key decides whether it is rederived.
- **0276:** `ProgramW` and `Placed`.
- **0103:** no caches beside rules; the analysis store lives as long as one pipeline.

It departs from LLVM's manual "preserves" set: invalidation comes from the recorded delta, so a pass cannot claim
a preservation it did not keep.

## Open

- **Morsels.** Whether a stateful `RulePass` (one numbering helpers, say) runs in morsels by a per-morsel state
  merged in order, or stays serial. The data decides, per pass.
- **Program writers' incrementality.** A program writer such as magic is maintained only through its analyses'
  rules (0173 §9's non-local passes) until those analyses are rules.
