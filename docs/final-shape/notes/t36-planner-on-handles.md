# t36: The planner on handles (decision 0173, stage 2 steps 4 and 5)

Status: work plan (2026-11-13). Kept current as each item is done: an item names its commit when it is.

## Intro

Decision 0173 makes a program a flat image read by handles. Stage 2 moves the planner onto it, and the user chose
how far (2026-11-13): **handles end to end**. A planned step names its literal by id, and every backend reads the
literal through the version's handles. No backend gets a decoded tree of its own.

This note maps what reads the planner's trees today and orders the work. Each item is a commit, checked by its
targeted tests and by the dl1 batch with images required (`RERAIL_PROGRAM_IMAGE=require`, the 48 findings equal,
cold and warm, profiling build).

## What reads the planner's trees

Surveyed 2026-11-13. Line numbers are at commit `9d88562d8`.

- **`Step`** (`plan.rs:190`: `at`, `lit: Literal`, `mode`) is the planner's only step type. Twelve other types
  share the name; none is on this path.
- **`Step.lit` outlives a run.** `Kept` (1140) holds steps, and `PLANS`/`OWN_PLANS` (1049, 1057) and
  `SESSION_PLANS` (1433) keep it across chunks and across a session's runs. `StoredPlan` (1220) already keeps
  `(at, mode)` and rebuilds the literal from the body (`kept`, 1263).
- **Readers of `.lit`:** the search and EXPLAIN (2197–3239), the SQL generator (3773–4114), `ensure_indexes`
  (4439), `observe` (5092–5118), `delta_sources`, `shown` and `same_order` (5287–5303), the join specs (5775–6016,
  6631–6764), `run` (9713–9725), `placement.rs` (303, 379), `replanning.rs` (28), `keeps::observe_steps` (1091),
  `emit.rs` (119), `sql_kernel::component` (98) and `compose::compile` (105).
- **The SQL generator** (`sql_select` 3712, `checked_query` 3724, `sql_query` 3743–4134, `sql_term` 3546) takes
  `&Program`, the head `&Atom` and `&[Step]`. It returns a `Query` that holds no trees.
- **The native set engine is tree-free.** `setfix` reads only `ComponentSpec`/`JoinSpec`. The lowering into them is
  in `plan.rs`: `join_spec_made` (5862, about 400 lines), `whole_join_spec`, `seed_specs`, `native_stratum_`, and
  the edge specs of `native_set_fixpoint` (7380).
- **`Model` holds no trees** and does not outlive a run. Its `input_set` identifies the inputs by address.
- **`Run`** holds `prog: &Program` and `numbers: *const Rule → index`. **`Driving`** (4267) holds `&Rule` and
  steps across waves and suspensions. **`Forms`** (7722) is an arena of per-stratum `Program`s.
- **Bodies that are not a rule's.** The planner plans a `(head, body)` pair, not a rule:
  - `native_set_fixpoint` plans `rest`, a rule's body less its recursive atom, under a made head `sfx_…` (7418);
  - `keeps` plans each `part` of a factored body under `…_part` (keeps.rs 634);
  - `plan_views` plans a view's one atom (8115).
- **Tree passes in or before `run`** each make a `Program`: `sql_kernel::normalize` and `stated_apart` (version
  forms built: `passes.rs`), `Lowered` (a clone, 8271), `keeps::choose`, `demand::split_negations`, the maintained
  forms (`m.form`), `lower`, `expand_memberships` and `maintain::rewrite`. `choices` keeps chosen programs across
  runs (`KEPT`, and the store's `.rr` text).

## The order

Leaves first. Each item leaves every caller working.

1. **A step names its literal by position.** `Step` loses `lit`. A plan is always read beside the body it was made
   for, so a reader takes `body[at]`. Plans in the caches hold no trees. The made-up bodies keep their own
   positions, so nothing changes in meaning.
2. **The planner plans a rule, not a pair.** `plan_rule` and its family take a `RuleRef`. The made-up bodies
   (`rest`, `part`, a view's atom) become rules of a run-local overlay, made by `Builder` (`rewrite` with `keep`,
   and `RuleBuilder`). Their steps then name literals of a real rule, as every other step does.
3. **The SQL generator reads handles:** `sql_query`, `checked_query`, `sql_term`, from a `RuleRef` and its steps.
4. **The join-spec lowering reads handles:** `join_spec_made`, `whole_join_spec`, `seed_specs`, the edge specs.
5. **`Run` is keyed by `RuleId`.** `numbers` goes. So do `Driving`'s `&Rule`s, `replanning::Watch`'s,
   `sql_kernel::component`'s, `compose`'s and `emit`'s.
6. **The tree passes become version edits:** `keeps::choose`, `split_negations`, the maintained forms (as views of
   the run's version, decision 0173 step 4), and `choices` keeping versions.
7. **`plan::run` takes a version** (step 5). Its callers pass the split's parts as views (E1), the run's edits as
   overlays (E2) and the session's cut as an overlay (E3). The trees on that path go.

## Done

(none yet)
