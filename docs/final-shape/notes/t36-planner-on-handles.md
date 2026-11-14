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

## Item 7, step by step

Surveyed 2026-11-14, at commit `f0ab6f6ab`. Every caller passes `run` a tree, and no corpus, typing or session code
makes a version yet. Inside `run` (about 1,680 lines) the rule lists are trees (`&Rule`, made into a `Body` only to
plan one, found by address), and the program's fields are read from the tree about 110 times. About 35 helpers read
only the tree. Eight have version forms already, unused on this path: `passes::normalize` and `stated_apart`, and
`analyses::accepts`, `needed`, `strata` and `projections`. `About::produced` and `Version::decl_of` are also unused
here. `Fields` covers most field reads.

Leaves first, as before. Each step leaves every caller working, and is checked against its tree form while it is
ported, over the targeted tests and dl1.

1. **The image holds the whole program.** `Again`, `together` and shapes, the gates' details (`level`, `src`,
   `quiet`, `deps`), `kindsets` and tapes become about rows, read by `About`. `Builder::of_program` stops refusing a
   maintained program, and `Placed::of_program` is whole for every program.
2. **`run` starts from a version.** It makes its input's version once, at the top, and normalizes it with
   `passes::normalize` and `stated_apart`, as derivations. The tree passes are decoded from the result (until step
   9). `Lowered`'s domains, ops and dialect travel beside the version, not in a cloned tree.
3. **The run's rule lists are ids.** The strata (`analyses::strata`), `rules_by`, `by_stratum`, `rules_by_stratum`,
   the strands and `issue`'s `Members` hold `RuleId`s (or `RuleKey`s), not `&Rule`s. `Versions::key` by address
   goes. `needed`, `projections`, `produced` and `accepts` read the version.
4. **The fields from the version.** `Run` holds the run's `Placed` and its `Fields`, not `prog`. `make_plan`,
   `Stats::read`, the `Run` helpers (indexes, deferred rows, `again`, the native relations), `sql_kernel::component`
   and `drive` read them there.
5. **The stratum readers on handles:**
   - `stratum_reads`, `first_reads`, `last_reads`, `retainable_stratum`, `with_answers`, `identities`, `plan_views`
     and `decoded_outputs`;
   - `native_stratum_`, `seed_specs` and `issue::Board`;
   - `native_fixpoint` and `native_set_fixpoint`, whose made rules (`rest` under `sfx_…`) become rules of a run-local
     overlay, as the keeps parts did.
6. **Compose on handles.** `compose::planned`, `compile` and `finish` read the version.
7. **The maintained forms by edits.** `maintain::rewrite_with` and `Driver::form` write their rules with `RuleBuilder`
   over the run's version, an overlay (E3 and E5). `supports` and `maintained_cross` follow, and the `Forms` arena
   goes.
   **Order, revised after steps 1 and 2 (2026-11-14).** The run's rule lists cannot become ids (step 3) before their
   readers read handles: each of about 30 helpers reads `r.head` and `r.body`. A tree rule of the run already finds
   its handle (`Versions::body(&Rule)`, by address). So the readers move first, one helper at a time, each reading
   the rule's `Body` and the version's fields (steps 5 and 4). Once none reads the tree, the lists become `RuleKey`s
   (step 3), which is then mechanical, and the address lookup goes.
8. **`run` takes a version.** Its signature takes the program's `Placed` and the lowering's domains. `run_atoms*`
   pass the version they were given. `choices` keeps only the version.
9. **The callers pass versions (E1 to E3).**
   - The split's parts are views with their inputs and outputs overlaid (E1). This needs a view whose fields are
     overlaid, and a fold that does not count a view's removals.
   - A run's edits are overlays (E2): the cross memo, the request cone and the run constants. This needs an input
     added alone, and a literal edit made as a remove plus an add.
   - A session's cut is an overlay (E3).
   - The prepared program read from the store keeps its image, rather than decoding it.
   The trees on that path go. The digests that still read the tree (`memo::program_digest`, `retain::key`,
   `choices::key`, `prepared_key`, `cross_digest`) are decision 0173's step 6. They keep a decode until then.

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
  - Step 2, the pricing: `choose` reads the program's version, its rules' bodies by index (`Read`), and the
    estimates, `calibrated*`, `observed_rule` and `observed_copies` take a `Body`: no rule is encoded alone. A
    relation's reader copies are one candidate overlay of the version (`inline::copies`, kept by no one), counted
    without being made (`inline::count`); a delta atom is found in a copy by its literal. Checked while porting: each
    copy's literals and price equalled the tree's (`unfold`) over the keeps, planner, corpus and session tests and
    dl1; the check and the tree's `unfold`, `copies` and `negated` are gone (the tests read the version's,
    `inline::unfold_rules`). dl1: the 48 findings are equal; cold 44.4 G, warm 1.8 G. Still to come in step 2: the
    version is the run's, passed in, not made from the tree (step 5), and `refusals`, `bounds` and `factorizable`
    read trees.
  - Step 3 in part: the split `split_negations` makes is an overlay of the program's version (`demand::overlaid`):
    each reader removed and its `pre_<i>` and itself over that added, each demanded relation's rules rewritten to
    read its demand first (its head's computed columns fresh variables, checked after), then the demands. The rules
    keep the tree's order. The choice of candidates, the guards and the types still read the tree, and the strata
    check is still `front::unstratified` over the decoded program (stratification over a version: step 1).
    Checked while porting: the overlay, decoded, equalled the tree's split over the planner's, demand, corpus and
    session tests and dl1; the check and the tree's split are gone. dl1: the 48 findings are equal; cold 43.6 G,
    warm 1.8 G.
  - Steps 4 and 5 (2026-11-14): the passes hand on versions, and the run reads the chosen one as it is.
    `body::Placed` is a program's version and where its rules are in it (rule `k` of the program is the version's
    `ids[k]`; in order where a version was made of the rules). `plan::run` makes the program's version once; `choose`
    edits it and gives its overlay, `split_negations` edits that, and `Versions::with` takes the last as the run's
    version 0: no rule is encoded after the first. A kept choice holds its version (`Chosen::version`): in memory the
    passes' overlay; read back from the store, the program's image (decision 0173), opened as it is. A choice kept as
    text alone has none, and the run makes its version (`Versions::of`). The decoded tree is still made, once per
    pass, for the rest of the run (item 7).
  - Checked while porting: the version taken, rule by rule, read as an encode of the chosen program (head, literals,
    observation key) over the planner's, keeps, demand, choices, cone, phase0, corpus and session tests and dl1; the
    check is gone. The choices test reads a kept choice back from the store and checks its image's rules likewise.
    dl1: the 48 findings are equal; cold 46.2 G (twice; 49.6 G after the hub merge, load about 75), warm 1.7 G.
  - Step 6 (2026-11-14): a maintained stratum's form, and its rules for deriving it again, are overlays of the run's
    version (`Versions::add_rules`, `Placed::over`): the rules they add, written over the program's atoms and
    relations, not a program image of their own. The form is still made by `maintain::rewrite_with` as a tree and
    kept in the `Forms` arena for the run's tree readers (item 7); its rules are encoded into the overlay once.
    Checked while porting: each form rule, read from the overlay, equalled the rules made alone (head, literals,
    observation key) over the maintain, session, edit, again, retain, cone, phase and corpus tests; the check is
    gone. dl1 (no maintained run): the 48 findings are equal; cold 46.5 G, warm 1.7 G.
  - Item 6, where it stands (2026-11-14). Every pass's output is a version edit: the inlining and the negation split
    are overlays of the program's version, the run reads the last one as it is, a kept choice holds its version, and
    a maintained form's rules are an overlay of the run's version. What still reads or writes trees, and why it goes
    with item 7 rather than here:
    - The passes' readings of the program: done below (the readings, 2026-11-14).
    - The maintained form's rules are written as trees (`maintain::rewrite_with`, over a one-stratum program cut from
      the base) and encoded into the overlay once. The rewrite is also the session's (E3, `retain::Cut::program`),
      which item 7 makes an overlay; it moves to `RuleBuilder` edits there, as `normalize` and `stated_apart` did
      (0173 stage 2, step 3). The `Forms` arena keeps the form's tree for the run's tree readers until then.
    - Each pass still decodes its result once for the rest of the run.
  - Item 6, the readings (2026-11-14): the passes read the program's version whole, not the tree.
    - The run's version is the program's image (`Placed::of_program`: `Builder::of_program`, its declarations and
      `About` too), marked `whole`. A program with `again`, `together` or shapes (a maintained one) is still a version
      of its rules alone (`Placed::made`), not `whole`; `choose` and `split_negations` give nothing for it, with a
      decision line saying so; no targeted test or dl1 run changed. Making the whole image cost about 9 ms of CPU a dl1
      batch.
    - `plan::fields::Fields` reads the program's other fields from a whole version: outputs, inputs, qualifiers,
      demand, gates, answers, natives, lattice, nullary, facts, arity. `About` holds the qualifiers, demand and gate
      keys for it.
    - `keeps::choose`: the strata (`program_image::analyses::strata`), `refusals`, `bounds`, the need, the estimates
      (a relation's width from its declaration in the version) and `factorizable` (its parts planned as overlays)
      read the version.
    - `demand::split_negations`: the candidates and each round's guards, splits and types (`demand::over`) read the
      version's rules by their handles, each variable by its name. The split's overlay declares each `pre_<i>` and
      `demand_<r>` and the flags, so it stays whole, and its strata are checked over it
      (`program_image::analyses::unstratified`, every failing component, as `front::unstratified`).
    - Checked while porting: each reading equalled the tree's (the strata as sets, the refusals, the rules by head,
      the bounds, the factors; the candidates, the refusals, the splits, the declarations and flags, the failing
      components, and every declaration of the decoded split in its overlay). This was checked over the planner's,
      keeps, inline, demand, negation, choices, cone, phase0, session and corpus tests and dl1, where 96 relations
      are demanded. The checks and the tree forms are gone; `keeps::factors` stays for `typing::file_local`.
    - dl1 (images required, profiling build, a fresh store): the 48 findings are equal; cold 44.1 G, warm 1.7 G
      (load about 65).
    - What reads trees in the passes now: nothing. They still take the tree too (`prog`): `choose` hands it to the
      planner's `plan_rule`, and both passes give a decoded program for the rest of the run (item 7). The maintained
      form's rewrite is item 7's (above).
- **Item 7, steps 1 and 2** (2026-11-14).
  - Step 1: every program has a whole image.
    - Join shape directives are about rows (`shape`, `shape_after`, `shape_bound`), written and read back by
      `program_facts`. `About` reads them, and reads the gates' details, the kind sets and the tape columns
      (`GateRef`, `ShapeRef`). `Builder::of_program` no longer refuses a program with shapes.
    - A maintained program's `again` and `together` are a typed form beside its version (decision 0215,
      `body::Maint`). Each stratum's rules and deltas are rules of an overlay of the version, and its relation sets
      are relation ids there. `Placed::of_program` is whole for every program (`Builder::of_program_less_maintenance`).
    - `keeps::choose` and `split_negations` give nothing for a program with `Maint`, as before. A program with shapes
      alone is passed again, as it was before item 6 made it a version of rules alone.
  - Step 2: `run` makes its input's version once, at its top, for every program (`Placed::normalized`).
    `passes::normalize` derives it and `passes::stated_apart` overlays it. The passes edit that version, and the run
    reads its rules from it. A run that reuses a choice whose program is its own reads it there too. The tree the rest
    of the run reads is still the tree passes' (`sql_kernel::normalize`, `stated_apart`): nothing is decoded.
  - Checked while porting:
    - the parts: the shapes, gates, kind sets and tapes, read back, and each `again` stratum's rules, deltas and
      relation sets, and `together`, equalled the tree's. Maintained programs of 1 to 16 strata derived again were
      checked, over the maintain, again, edit, retain and corpus tests.
    - the normalized version: equal, rule by rule (head, literals, observation key), digest and about rows, to the
      normalized tree's image. This was checked over the planner's, keeps, demand, negation, inline, choices, cone,
      phase0, session, maintain, again, edit, retain, corpus, rr and image tests and dl1.
    The checks are gone.
  - Cost: making the version is a span of its own ("the program's version"). On dl1 cold it is 46 ms of CPU over
    30 runs, of the batch's 3.8 s in spans. Before step 2, a corpus run made its image only when it missed the kept
    choices. The callers' versions (step 9) remove it.
  - dl1 (images required, profiling build, a fresh store): the 48 findings are equal. Cold took 47.0, 49.1 and
    50.1 G over three runs (load about 50), within the spread of this session's runs (44 to 50 G). Warm took 1.7 G.
- **Item 7, step 4 begun: the fields the run reads, from the version** (2026-11-14).
  - A version's fields are read once and shared: `Placed` holds a `FieldsData` (its `About` and its inputs and
    outputs) made at the first `Fields::of`, shared by its clones, and `Fields<'v>` borrows it.
    `Versions::fields(k)` gives the run's `k`-th version's (the program's is 0).
  - `About` keeps every native's fields (`NativeSpec`, one per `front::Native` variant), a lattice's `ty`, `merge`
    and `widen`, and the demand in order. `Fields` answers by name every program field `run` and its helpers read of
    the tree: inputs and outputs in order, declarations and their columns, facts, lattices, natives, answers in order,
    gates, qualifiers, tapes, kind sets, nullary, demand and shapes. Not `again` and `together` (`Maint`).
  - Not read yet: `run` still reads the tree. Left for its port: the digest over the fields (`plan.rs` near 3602) and
    the keys made of the tree's text (near 7340, 7373), which need a version's form; `Stats`' input set, keyed by the
    tree's address (near 1006); and the tree edits (`stated_apart`, an outputs reset near 9594), which the passes
    already make on the version.
  - Checked: `a_versions_fields_say_what_the_tree_says` (engine, `program_image::tests`) reads every field through
    `Fields` from `Placed::of_program` and compares it by text with the tree, on the sampled pack programs and on
    programs with a field of every kind (`every_field`: each native kind, merges and widenings, answers, gates,
    qualifiers, tapes, demand in order). `fields_are_read_once_and_say_the_trees` (logic) checks the sharing.

- **Item 7, step 4: the run reads the program's fields from its version** (2026-11-14).
  - `Run` holds `version`, the place of its current program among `run.versions` (0, the program; a maintained
    form's once added), set with `prog` by `set_prog` wherever the run swaps to a form and back, and in a strand of
    forms (`run_forms`, `strand_forms`). `Run::fields()` reads the current program's `Fields`; a helper that mutates
    the run takes them from its own clone of `versions`.
  - `Versions::add(form)` adds a whole version of its own (`Placed::of_program`: its rules, fields and `Maint`),
    found by the program's address (`index_of`); `add_rules` stays an overlay for again's rule lists. The run's
    version 0 is whole: a kept choice stored as text (no version) has its image made at the run's start, and a
    program with no image is an error.
  - Moved to `Fields`: the tables made over the declarations (iterated by name, `decls_by_name`, facts from the
    version's cells), the named set, inputs and outputs, adopted inputs, the gated and kept sets, the retained strata's
    checks (`retainable_stratum`, `with_answers`, `kept_strata_ran`, `produced_rows`, `kept_strata_hold`,
    `stored_types`), the requests and produced relations (`produced`, `request_of`: of several requests the last by
    name, as `answers::produced` maps it), the natives (`native_relation`, `reach_rows`, `native_reach`, the Python and
    components operators take `NativeSpec`), `index_for`, `insert_rows`, `strand_reads_with`, `strand_forms`,
    `absorb_forms`, `seed_specs`, `native_stratum_` (lattices, gates as `GateRef`s sorted by level then name,
    qualifiers, facts, types), `drive`, `native_fixpoint`, `native_set_fixpoint`, `decline_class`, and
    `sql_kernel::program_for` and `component` (lattice tops by name, value types, inputs). `issue.rs` reads no field.
  - Still the tree's, and why: `again` and `together` (step 7, `Maint`); the rule lists; `Model`'s methods, which
    take the tree (`plan_rule*`, `sql_select`, `make_statistics`, `late`, `resolve`, `fork`, `absorb`), and
    `prior_stats`/`prior_now` with them; `join_spec` (its own `Run` impl, with `prog_types`); the stratum readers
    shared with tree callers (`stratum_reads`, `first_reads`, `last_reads`, `identities` and `leaf_identity`, whose
    digests are of the tree's text) and those shared with `emit` (`plan_views`, `decoded_outputs`): step 5; the run's
    input set extended with a form's inputs (its names must live as long as the run); and what runs before the
    version is made (`sql_kernel::normalize`, `stated_apart`, the lowering's domains, the composed path).
  - Checked while porting: at the run's start and at each `set_prog`, every field the run reads (inputs and outputs
    in order, declarations, facts by text, lattices, natives and their reads, answers in order, `request_of` against
    `answers::produced`, gates, qualifiers) equalled the tree's, over the planner's tests (logic `plan::`, 110) and the
    engine's keeps, demand, negation, inline, choices, cone, phase0, session, maintain, again, retain, edit,
    `corpus::`, `rr::`, program_image, strata, answer, native and setfix tests, and dl1 cold and warm. The check is
    gone; `fields_are_read_once_and_say_the_trees` checks a form's version (`add`, `index_of`).

- **Item 7, steps 4 and 5 in part: the Model's fields and the stratum readers on handles** (2026-11-14).
  - The Model reads the program's `Fields`, not the tree: `make_plan` (from the run's `Placed`), `Stats::read` and
    `input_stats`, `make_statistics`, `late`, `resolve`, `fork` and `absorb`, `is_input`, `select.rs`'s `relation`
    and `declared`, `prior_stats` and `prior_now`, and `join_spec`'s value types. `plan_rule*` and `explain_order`
    take the fields beside the rule's `Body`; `view_stats` takes the view's `Body`.
  - The stratum readers read rules by their handles in the run's `Versions` (`body_of`): `stratum_reads`,
    `first_reads`, `last_reads`, `identities` and `leaf_identity`, `survivors`, `cyclic_relations`, `solving_of`,
    `recursive`, `reads_change`, `read_by`, `runnable`, `retainable_stratum`'s rule test, `plan_views` and
    `decoded_outputs`, and `Model::wants_strata`. A caller holding only a tree (compose's trimmed copies, the kept
    strata's and the cut's identities) makes the version for them (`identities_of`, `Leaves`, `survivors_alone`).
  - The identities stay digests of the tree's text (decision 0103), now written from the handles (`plan::tree_text`:
    a mirror of each tree type whose derived `Debug` writes what the tree's writes).
    `the_tree_text_from_handles_is_the_trees` (engine, program_image) checks it byte for byte, for every rule and
    every relation's fields of the sampled pack programs, `every_field` with a native of each kind, and `analysed`.
  - Still on the tree: `native_stratum_`'s own rule reads (the empty-delta and unfilled filters, the gates' readers,
    the waiting reads), `seed_specs`, `issue::Board`, the fixpoints' made rules, the again and together paths, the
    composed path and the rule lists themselves (step 3); `prog.` field reads outside the run (the compose path,
    `emit`'s callers).
  - Checked: logic `plan::` (110), the engine's keeps, demand, negation, inline, choices, cone, phase0, session,
    maintain, again, retain, edit, `corpus::`, `rr::`, program_image, strata, emit and answer; dl1 against
    589b75d43's build: the same 48 findings cold and warm.
  - Measured on dl1 (profiling builds, same load): the port first cost about 2 G instructions cold (45.8–46.3 G
    against 43.5–44.3 G). Two causes, both fixed: the kept frames' readers (`Retained::from_frames`,
    `kept_strata`) made the tree's version twice, for its identities and for its leaves (`Leaves` now makes it
    once and gives both, `Leaves::identities`); and the readers that walk a rule once (`stratum_reads`, under
    `first_reads` and `last_reads`; `cyclic_relations`, `runnable`, `solving_of`, `retainable_stratum`) made a
    `Body`, which collects the rule's literals, for every rule they read. They read it in place
    (`Versions::rule_ref`). After: 43.8–45.5 G cold, 2.6 G warm, as before.
  - Open (found while building, not this item's): the crate split left about twenty warnings of dead code and of
    private types in public interfaces in the store, prof, java_lean and w crates (items only another crate's tests
    use); the logic and engine ones were fixed here.

- **Item 7, step 5 continued: the run's rule readers on handles** (2026-11-14).
  - Read by their handles in the run's `Versions` (`rule_of`, `reads_of`: a rule read in place, with no `Body`):
    `strand_reads_with`, `run_members`' waiting reads, `native_stratum_`'s filters of rules over empty or unfilled
    relations and its gates' readers, the planning spans, the answering strata's reads, the probes of
    `native_set_fixpoint`, `issue::Board`'s estimate and digests (`Body::observed_key`, which equals `obs_rule`), and
    the run loop's: the named relations, the pinned views' reads, a form's count requests, its digests and estimate,
    its plainness, the deferred reads, the SQL strata's reads, the answered relations' readers and the rows trace.
  - Still on the tree: the again and together paths (`g.rules`, `g.deltas`, their last reads: step 7, `Maint`), the
    fixpoints' made rules (`native_fixpoint`, `native_set_fixpoint`'s edge parts, to become a run-local overlay), the
    literal counts charged before the version is made, and the rule lists themselves (step 3).
  - Checked: logic `plan::` (110); the engine's keeps, demand, negation, inline, cone, phase0, session, maintain,
    again, retain, edit, strata, answer, native, setfix, `corpus::` (in chunks) and `rr::`; dl1 against 80ffec69e's
    build: the same 48 findings cold and warm.
  - Measured on dl1: 45.1–45.7 G cold against 44.2–44.3 G, 2.6 G warm on both. The difference is the lookup of a
    rule's handles (`Versions::rule_ref`, about 6% of `run`'s samples): the rule's address searched among the
    versions' ranges, then `Version::rule` down the overlay chain (one binary search a pass's overlay). Step 3 (the
    lists as `RuleKey`s) removes the address search; folding a run's overlays into one (the builder's fold, which
    counts removals today) removes the chain.
