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

- **Item 1, a step names its literal by position** (2026-11-13).
  - `Step` is `{ at, mode }`, `Copy` and `Eq`. A plan is read through `Order` (a body and its steps), with `lit`,
    `iter`, `lits` and `take`, so no reader indexes a body itself.
  - The plan caches (`Kept`, `PLANS`, `SESSION_PLANS`, `Priced`) hold no trees.
  - Every reader takes an `Order`: the SQL generator, the join specs, `ensure_indexes`, `observe`,
    `observed_fans`, `delta_sources`, `shown`, `placement`, `keeps::observe_steps`, `emit`, `sql_kernel` and
    `compose`.
  - Each seed's order carries its rule (`Driving.orders`), as do `run`'s planned rules and the set fixpoint's
    seeds. The made-up bodies (`rest`, the parts) are kept beside their plans.
  - A kept plan is no longer compared literal by literal with its body. Its key (a digest of the body) already
    names it.
  - The coverage ledger (`gate/executor-coverage`) records the clearer message: "no SQL for @printf($a, $b)"
    rather than a dump of the steps.
  - Tests: the planner's (53 pass, 5 ignored), the corpus planner's (77), the session planner's and emit's (12),
    and the coverage ledger's (2).
  - dl1 (images required, profiling build, a fresh store): the 48 findings are equal. Cold took 53.6 G
    instructions (54.9 G before) and warm 19.3 G (19.8 G): steps no longer clone literals.
- **Item 2, the planner plans a rule** (2026-11-13).
  - The planner reads a rule through `plan::body::Body`: the rule's handles in its version, its literals by position,
    each literal's variables as a set of slots (`VarSet`). `plan_rule` and its family (`_priced`, `_explained`,
    `_reused`, `_live`, `explain_order`), the search, its keys and its sizes take a `Body`, not a (head, body) pair.
    The search's bound sets are slot sets and its subset masks dense slot bits, so no set of names is made.
  - `Body`'s readings are the tree's, stated over handles: `vars`, `ready`, `cmp_binds`, `is_join`, constants, the
    digests (`plan_key`, the observations' key) and the text (`Version::show_*`). A test checks each against the tree
    on the pack programs, so the keys of kept and stored plans are unchanged.
  - Where the rules come from, until `run` takes a version (item 7): `body::Versions` holds the run's programs' rules
    encoded (the program's at `run`'s entry, each maintained form's as it is placed, a form's rules for deriving it
    again when that is chosen), and finds a rule by its place in its program. Encoding costs about 1 ms a run on dl1
    (32 ms over 124 cold runs, 22 ms warm). `compose` and `emit` hold their program's; `keeps` prices each rule alone
    (its copies are of no program).
  - The made-up bodies are rules of versions of their own (`body::made`): the set fixpoint's rest under its edge
    table's head, the lattice fixpoint's parts, and keeps' factored parts. A view's atom is not planned (`plan_views`
    only prices its statistics), so it needs none. The made rules are built from trees, as their readers (the SQL
    generator, the join specs) still are; items 3 and 4 build them from handles.
  - Tests: the agreement test (`the_planners_reading_of_a_rule_is_the_trees`), the planner's (54 pass, 5 ignored), the
    corpus planner's (77, 10 ignored), the image and text tests and the planning tests (27), the coverage ledger's (2),
    and the logic crate's choices, keeps and prepared tests (10).
  - Seen once: `phase0::tests::bootstrap_resolves_as_the_builtin_reader_does` failed in one of three runs of the corpus
    set, and passed alone and in the other two. It plans nothing; open.
  - dl1 (images required, profiling build, a fresh store): the 48 findings are equal. Cold took 57.4 G and 55.9 G
    instructions in two runs, warm 20.2, 20.0 and 20.15 G (item 1: 53.6 G and 19.3 G, one run each). Sampled, encoding
    is 0.5% of a cold run and 0.2% of a warm one, and the whole search 1.7% of a cold run, so most of the difference is
    the runs' own spread, not item 2's cost.
