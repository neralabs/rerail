# 0179. A cold run's preparation computes what changed

Status: built (2026-11-13). Follows decision 0178 (the cold passes on the pool).

## Intro

After decision 0178, the cold `dl1` batch (one Python file, 44 data-layer policies run as one pack) was still mostly
serial on the main thread. Decision 0178 listed five levers that remained. This record goes through each one.

The common finding: most of the work left was not serial by nature. It was **repeated work** that an earlier change
should have avoided:
- incremental splits that recomputed half the program;
- trial checks that rebuilt the whole program to test one literal;
- fixpoints that re-checked rules none of whose inputs had changed;
- one regex that case-folded all of Unicode.

Every change below gives the same result as before. The findings of `dl1` are identical, and so are the checks that
compare against the whole computation (`RERAIL_SPLIT_CHECK=1`, `RERAIL_MAGIC_CHECK=1`).

## Decision

### The split computes only what a round changed

Pushing file-local joins (`push_with`) tries all rules' helpers in one round, splits the program to check them, and
retries the rules whose helpers failed. Each retry's split is incremental (`split_memo`): it recomputes only the cone
of the relations that changed. But every round recomputed about 1,100 of 2,600 relations.

**Cause:** each helper was named `fl<n>_<head>`, where `<n>` was the program's declaration count at the time. When a
rule's helper failed and was dropped, every later helper got a new number, so every later helper looked changed.

**Fix:** a helper's name is now kept per (rule, component) across the rounds. The retry rounds' cones went from
1,122 / 1,059 / 898 / 855 relations to 257 / 92 / 157 / 92.

Two more whole splits went incremental:

| split | what forced a whole split | now |
|---|---|---|
| after factorizing (2,489 relations) | a relation that stops being derived but keeps its declaration changed the key shared by every relation | each base relation's declaration is its own key, so only its readers are recomputed: 330 |
| the last one (2,960 relations) | unread inputs were dropped, which changed the shared key | the key holds only the inputs a split asks about (derived relations, facts, phase exports): 776 |

A fixpoint round's next relations were also gathered into a `BTreeSet` with one entry per reading rule, which meant
sorting tens of thousands of names to keep a few thousand. They are now deduplicated through a hash set first, then
the unique names are sorted (`RuleIndex::heads_reading_any`).

### Magic adornment's trials are checked incrementally

`magic_adorn` keeps each optional literal of a magic rule only if the program stays stratified. Each trial used to
rebuild every magic rule and stratify the whole program.

Now a trial rebuilds only its own rule and compares that rule's edges before and after. The last whole check's
components are listed dependencies first, and a new edge leaves them as they were when it either:
- goes to an earlier component, which cannot reach the head, or
- is a positive edge inside the head's own component, and no negation closes a cycle in that component.

When every new edge is like that, the program is still stratified, and the literal is kept without a whole check.
Every other literal is checked whole, as before, and so is every literal when negations are gated. With
`RERAIL_MAGIC_CHECK=1`, every trial is checked whole too, and the run fails if a literal kept without a whole check
would have unstratified the program.

### Fixpoints skip rules whose inputs did not change

Two key-finding loops re-proved every rule in every sweep:
- the program analysis (`ProgramAnalysis::analyze`), for relation keys;
- factorizing's relation keys (`relation_keys`).

Each now re-checks a rule (or a column) only when a relation it reads has gained a key since it was last checked.
The sweeps and their order are unchanged. A skipped rule would have given the same result, so the keys found are the
same.

### Per-rule passes on the pool

These passes each rewrite or analyse one rule at a time, so they now run in parallel:
- simplify's per-rule rewrites (`concat_inverses`, `fold_constants`, `merge_keyed`, `wild_singletons`,
  `drop_kind_tests`, `drop_implied`, `existence_as_reads`);
- the program analysis's per-rule analyses.

Lowering's column lookup (`rule_vars`) found each atom's columns by name once per argument, on every pass. It now
looks them up once per atom.

### A class of every character is any character

One policy's `rx_span` is `(?i)…CAST\([\s\S]{0,160}?…`. Under `(?i)`, the regex compiler case-folds each class, and
`[\s\S]` is all of Unicode. That fold took about 0.2 s of main-thread time per batch (`SimpleCaseFolder::mapping`).

`span_prefilter::unfolded` now writes `[\s\S]`, `[\w\W]` and `[\d\D]` (either way round) as `(?s-i:.)` when they appear
outside a class and unescaped. That matches the same characters and skips the fold. Patterns that turn Unicode off
(`-u`) are left as written. Both the regex and its prefilter compile the rewritten pattern, and a batch's patterns
compile in parallel.

## Measured

`dl1`, cold (`RERAIL_TAPE_CACHE=off`), on `--profile profiling` builds. Seven runs of each, alternated with the build
before decision 0178, at load 52–63:

| | before 0178 | after 0179 |
|---|---|---|
| wall | 1.96–2.36 s (median 2.32), one outlier at 4.73 | 1.59–2.11 s (median 1.77), one outlier at 4.80 |
| instructions | 27.0–27.25 G | 25.7–26.2 G |

Decision 0178 alone measured a median of 1.93 → 1.69 s at load 32–42.

## What stays, and why

- **The order of the steps, and the nested typing run.** Phase 0, combining, the bridged typing run (about 0.3 s),
  phase 1 and prepare run in that order. Each needs either the step before or the combined program: the typing run
  needs to know which outputs the combined program reads. Overlapping the nested runs means running an engine run
  on another thread. The run's state lives in thread-locals: 123 `thread_local!` in the engine, holding run
  nesting, phases, the profile, sessions and the kept plans. The split memo is process-wide too. That state has to
  be passed explicitly before nested runs can overlap safely. It is a refactor of its own, not a change to this
  path.
- **The negation split's rounds** (3 and 5 on `dl1`). Each round already takes every failure it finds. The rounds are
  a real fixpoint: dropping a candidate changes which readers anchor the others.
- **Lowering's domain fixpoint.** The first rule to give a column its domain wins, so the order matters, and running
  it in parallel would need a proof that every order agrees.
