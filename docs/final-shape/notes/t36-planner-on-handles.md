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

## Item 6, step by step

The three passes in `plan::run` that rewrite the program (`keeps::choose`, `demand::split_negations`, a maintained
stratum's form) each clone the tree and edit the clone. Under decision 0173 §7 each is an **overlay** of the run's
version: the base rules it removes and the rules it adds, each added rule recording its source. A retry (the demand
split's rounds, a refused inlining) is a candidate overlay, dropped when refused.

1. **The edit tools.** `RuleBuilder::lit_with`: a literal of one rule copied into another through a `Rewrite`
   (renamed variables, substituted terms, an atom negated, a comparison's operator turned), which unfolding and the
   demand split need. Stratification over a version (`front::unstratified`'s check), for the split's refusal.
2. **`keeps::choose` over the run's version.** Its readings (`rules_of`, `readers`, `refusals`, `bounds`, the
   estimates and the pricing) read the version, and price program rules by their handles, not by `alone` encodes.
   The copies it prices and the program it chooses are overlays: each reader copy written from the reader and the
   inlined rules (`unfold`), the inlined relations' rules removed.
3. **`split_negations` over a version.** Each round's split (`pre_<i>`, `demand_<r>`, the demanded rules' reads) is
   an overlay of the version it was given, checked for strata over the version.
4. **`choices` keeps versions.** A kept choice is the chosen overlay (its bytes over the run's base, which
   `choices::key` names), in memory and in the store.
5. **The run reads the chosen version.** `body::Versions` takes it as it is, with no encode; the tree the rest of
   the run reads is decoded from it once (until item 7).
6. **The maintained forms as views.** A stratum's form (`Driver::form`, the `Forms` arena) is a view of the run's
   version with its rules overlaid, not a program of its own.

Each pass is checked against its tree form while it is ported, as items 3 and 4 were: the overlay, decoded, equals
the tree pass's program, over the planner's, corpus, session, `rr` and keeps tests and dl1. Then the tree form goes.

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
- **Item 3, the SQL generator reads handles** (2026-11-13).
  - `plan::select` makes a planned rule's SELECT from its `Body` and steps: `sql_select`, `checked_query` and
    `sql_query` take `(prog, &Body, &[Step], delta_at, tables)`. Variables are bound by slot (`Env`); a walk's CTE
    columns are still named by the variables, in name order, so the SQL text is unchanged.
  - Every caller passes the rule it planned: a stratum's seeds and probes, the set fixpoint's seeds, probes, reprobes
    and edge rests (each edge keeps its made version), the lattice parts, the views (`plan_views` takes the run's
    `Versions`), `emit`, `compose` and `sql_kernel`.
  - Checked while porting: each SQL request also ran the tree generator on the rule decoded from the version
    (`program_image::Tree`) and asserted the same text or the same error. It never disagreed over the planner's,
    corpus, emit, session, coverage, `rr` (127), native, image and text tests, the convert replay sample (268 events of
    137 checks) and dl1 cold and warm. The check and the tree generator are gone. The tree `sql_term` stays for one
    reader, the join specs' constant conditions (item 4).
  - dl1 (images required, profiling build, a fresh store): the 48 findings are equal; cold 55.7 G instructions, warm
    19.9 G (53.6 G and 19.3 G in the run with the check on: the runs' spread).
  - The convert replay sample (`a_sample_of_the_replay_gives_the_recorded_findings`) took 345 s in the debug test
    binary, past the 170 s watchdog. Two causes, both fixed (2026-11-13), and it now takes 33 s:
    - `corpus::session::current` held the process's session lock for a whole corpus run even when the run was no
      session's, so the sample's eight workers ran one at a time (one core busy, the workers in `__psynch_mutexwait`).
      A run that is not the session's holds nothing now. The workers are named as a test's threads, so each has a
      store of its own (`test_store::own`).
    - One check (`serverplan_state_tripwires`) took 198 s alone: an aggregate's demand
      (`rr::aggregate_demand::guards`) left out a guard that reaches the key only through a comparison
      (`spt_flow_jb(rr_t1, 92)` with `rr_t1 = @sub(p, 1)`), so every byte of a 6 KB text was demanded and its helper
      held every pair of bytes (17.9 M rows, indexed three times). Guards and comparisons now connect in one fixpoint;
      the check takes 11 s. `RERAIL_REPLAY_CHECK` runs the checks whose group or name holds it.
- **Item 4, the join-spec lowering reads handles** (2026-11-13).
  - `plan::join_spec` lowers a planned rule (`Body` and steps) into a `JoinSpec`: `whole_join_spec`, `join_spec` (the
    run's kept spec, under the same digest: `Body::digest_atom` and `digest_lit` write what the tree's did) and
    `join_spec_made`. A spec's slots are keyed by what they hold (a rule's variable by slot, a made value by its
    step), numbered as first met, so the specs are unchanged; a made value no longer shares a name space with the
    rule's variables (`rr_fn<i>_<k>` could have been a variable's name).
  - A prefix is probed with `bare` (the steps alone, a head of no terms) instead of a made head atom. The decline
    messages (`missing`) read the body's text; `decline_class` still reads the tree literal (item 5 moves it).
  - Every caller passes its rule's `Body`: a level's seed joins, a stratum's seeds, probes, whole plans and replans,
    and the set fixpoint's probes and reprobes.
  - Checked while porting, as item 3 was: each made spec was compared with the tree lowering's for the rule decoded
    from the version, over the planner's, corpus, emit, session, coverage, `rr`, native and image tests and dl1 cold
    and warm; they never differed. The check, the tree lowering and the tree `sql_term` are gone.
  - dl1 (images required, profiling build, a fresh store): the 48 findings are equal; cold 55.1 G instructions, warm
    19.8 G (56.8 G and 19.3 G with the check on).
- **Item 5, the run holds rules by key** (2026-11-13).
  - `body::RuleKey` names a rule of the run: its version (its place in the run's `Versions`; the program's is 0)
    and its id there. A rule's number is its id in version 0 (`RuleKey::number`), so `Run::numbers` (a map by
    address) and the position scans for rule numbers are gone.
  - `Order` holds the rule's `Body` beside its steps. Its readers read handles: `ensure_indexes`, `observe`,
    `observed_fans`, `delta_sources`, `shown`, keeps' `pattern` and `observe_steps`, the device placement's
    `Shape::add` and `add_joins` (the head's terms read from the body), emit's reads and recursion test, and
    `decline_class`.
  - `Driving` holds keys (`orders`, `probes`, `seeds`), and its driver reads each line's body, found once when it
    starts. `Driving`, `Native` and `Asked` no longer borrow the program. `replanning::Watch` is made from the
    probes' bodies, `sql_kernel::component` takes the planned rules by key, and `emit` and `compose` read a
    stratum's rules by key (`RuleKey::program`).
  - Still trees, for items 6 and 7: the run's per-stratum rule lists (`rules_by`, the strands, the forms), the
    stratum's filters before its plans (gates, survivors, empty reads), and the set fixpoint's probes, held only
    within one call. A key is found from such a rule by address (`Versions::key`) until `run` takes a version.
  - dl1 (images required, profiling build, a fresh store): the 48 findings are equal; cold 55.5 G and warm 20.2 G
    after the `Order` step, 56.0 G and 20.0 G after the keys (the spread between runs is about ±1.5 G).
- **Item 6, in progress** (2026-11-14).
  - Step 1 in part: `RuleBuilder::lit_with` copies a literal of one rule into another through a `Rewrite`, and
    `Rewrite::var` renames a variable where no term is asked for it (`nearest`'s). Stratification over a version is
    still to come (step 3 needs it).
  - Step 2 in part: the program `choose` gives is an overlay (`keeps::inline`): each pass an overlay of the one
    before, a reader replaced by its copies (its literals before the read, the def's renamed, the head's equalities,
    its literals after; a negated read by the product of its defs' negations), then the inlined relations' rules
    removed. The copies keep the tree pass's order, names and origins, so their observations match. A fold
    (`Builder::finish` folds an edit of half its parent) renumbers the rules; the pass maps them.
  - Checked while porting: the overlay's rules, in order, and its origins and unfinished relations equalled the tree
    pass's (`unfold_reads`) over the keeps, planner, corpus and session tests and dl1 (4,036 rules inlined across its
    runs). The check and `unfold_reads` are gone. The run still reads a tree, decoded from the overlay (step 5), and
    `choose`'s pricing still reads trees (`unfold`, `copies`).
  - dl1 (images required, profiling build, a fresh store): the 48 findings are equal; cold 45.4 G, warm 1.8 G (46.0 G
    and 1.8 G after the hub merge).
