# 0281: Program passes run under a pass manager: rule rewriters and program edits over cached, maintained analyses

Status: accepted (2026-11-15); step 1 in progress (see Path).
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

- **Cached by version.** An analysis is kept per version digest for as long as a pipeline runs. It is the
  pipeline's state, dropped when the pipeline ends. It is not a process map, so 0103's rule (no static memo maps,
  caches are rules) holds: what outlives a run is a fact, kept by the steps' keys (§6).
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
     dense `IdSet`/`IdMap` columns); `About`, `Strata`, `Needed` and `FieldsData` as analyses; `normalize` and
     supports as rule passes, `stated_apart` as a program pass. Tests: `a_rule_pass_costs_the_rules_it_selects`,
     `analyses_carry_over_edits_that_miss_what_they_read`, `a_pass_that_writes_for_a_kept_rule_fails`, and the
     passes' own.
   - As built: `RulePass::finish` takes the `Builder` itself (no separate `FieldsW`); `Delta` is read off the
     version the writer made (an overlay is its delta, a view its ranges; fields by their hashes); a derivation that
     changes nothing returns its input, so its analyses carry. A rule pass selecting all rules runs as a derivation
     (the program's order kept); one selecting fewer, as an edit.
   - Not yet: the gate, the split, `RoundProgram::of`, `qualify` and the cut; morsels (§3); re-running a rule pass
     on a delta alone (§3), which needs a stored step to re-run from (step 6).
2. **The analyses** behind the trait: `Fields`, `Heads`, `Readers`, `Strata`, `Components`, `Needed`, `Domains`, and
   the split's columns. Each is checked by the debug build's recompute-and-compare.
3. **0276 step 3's preparation passes** as `ProgramPass`es: `locate`, the merges, `demand_exports`,
   `factorize_exports` and `push_local_joins`.
4. **0276 step 4's front passes** in the pipeline: `prune`, `simplify`, `reach_form`, `magic`, the matchers and
   the strings. Each is checked against its `TreePass` by digest, and the `TreePass` is deleted once it agrees on
   dl1 and the pack.
5. **The per-clause half of `rr::lower`** as `RulePass`es over the tape's clauses (0276 step 5).
6. **Steps keyed (§6),** then analyses as rules (0225 §5), one at a time behind the trait.

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
