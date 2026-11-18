# 0354. The join estimates' learned correction, in shadow

Status: step 1 built (2026-11-17): samples kept, the model trained and compared, not used. Work queue Q82. Builds on
decisions 0100 §14 (the learned cost model: `rerail-forest`'s regression forests, the cost store) and 0203 §2 (the
batch prediction a round of choices asks: `joins::Pricer`).

## Intro

The planner prices each step of a join's order with a rule of thumb, named in the step's record as its *basis*:
postings, field statistics, kind frequency, containment, membership. Rules of thumb fail where values correlate, and
on hogql the planner's planned work was off by 2.2× (weighted) per stratum, some strata by hundreds of times (Q44).
Each correction added since has been more planning work: exact pair statistics cost 74 ms a measurement, and
`p278_names_class`'s four cost 1.14 s against 1.33 s for the whole stratum.

The user (2026-11-17): "is it time for us to roll out the classifier we built? We are starting to hit a point where
the cost of the extra planning work required is exceeding the cost of the joins it would be improving." Agreed, in
three steps: (1) in shadow, predicting and compared against the planner, no plan changed; (2) in place of exact pair
statistics at a slow flag (0203 §3); (3) in every round's choice (`joins::Pricer`).

## Decision (step 1)

1. **A step is a sample.** At the end of each component, each line that ran one order, with a plan record of that
   order, gives a sample per step after its first (`costs::join_steps`): what the planner knew when it priced it
   (its basis, how it reads and what, the columns bound, its place, the planner's rows in and fanout, the relation's
   rows, whether the relation is being solved, seed or probe) and what the step read and made.
2. **The target is the step's own miss**: ln((made + 1) / (planned fanout × rows read + 1)). The planned fanout is
   applied to the rows the step actually read, so a step is not blamed for the steps before it.
3. **Kept as the executor's samples are**: a table of the cost store (`join_steps`), written by the cost statistics'
   writer in its transaction, at most 400,000 a machine. The queue the driver submits to is listed as state in
   `facts_only` (as `sql_plans`' is).
4. **The model is a regression forest** (`rerail_accel::join_model`, on `rerail-forest`, the cost model's
   configuration), each step weighted by the log of its rows of work. A step's work is what it read and the more of
   what it made and what it was planned to make: weighed by what it made alone, an overestimate weighed little,
   though it misleads a plan as much (`up_fn_of` was priced at 1,000 times its work, and the GPU found cheaper).
5. **Compared, not used**: trained on all but the newest fifth of the samples, the corrected and the planner's
   estimates measured against what the held-out steps made (`join_model::compare`). It wins where it misses less
   weighted by work and no more at the median. Nothing is installed.

## Measured

hogql (437 files, 317 rules, cold, profiled, m3): 336,397 steps from one run (1.88 G rows of work). The newest
200,000 read; trained on 160,000 in 2.2 s (2.5 s CPU, two threads); compared on the newest 40,000:

| | planner | model |
|---|---|---|
| weighted by work | ×1.65 | ×1.42 |
| rms | ×3.07 | ×2.21 |
| median | ×1.02 | ×1.04 |
| within 2× | 83% | 84% |

| basis | steps | rows of work | planner | model |
|---|---|---|---|---|
| filters (none) | 21,492 | 42.7 M | ×1.50 | ×1.30 |
| postings | 13,629 | 13.9 M | ×1.85 | ×1.60 |
| kind frequency | 2,286 | 3.0 M | ×3.81 | ×2.91 |
| exact pair | 418 | 1.5 M | ×2.34 | ×1.42 |
| membership | 452 | 1.1 M | ×1.06 | ×1.40 |
| field statistics | 1,525 | 0.4 M | ×1.14 | ×1.12 |
| subtree size | 140 | 0.2 M | ×5.31 | ×3.78 |
| relation size | 58 | 0.04 M | ×6.56 | ×4.77 |

By the gate (item 5) it does not win yet: its median miss is ×1.04 against ×1.02. It misses less weighted by work
(×1.42 against ×1.65) and on every basis but membership.

## Across corpora, and the orders not kept (2026-11-17)

Asked (the user): does the model know what kind of estimate it corrects (a membership), and can samples be made where
it is wrong?

- **What it knows.** The basis names the planner's rule, the access a membership test. Added: the bound key's most
  distinct values (`key_ndv`, the relation's statistics over the columns bound when the step reads), and the
  literal's identity (rule digest and place). The literal is kept in the table, not a feature (below).
- **The orders not kept.** A line whose order was replaced (0206's handoffs) gives each order it ran, by its own
  plan record and counts (origin `replaced`); a trial that ran to its end gives its order's steps (origin `trial`,
  `TrialRun::steps`). Each step's origin is stored, and the run (`run`: the process), so a run is held out whole.
- **Across corpora.** corpus0 (300 files) and hogql (437), cold, profiled, on the m3, each pair into one store in
  both orders; the model trained on the first corpus's steps and asked on the second's (`COSTS_HOLD=run`):

| trained on, asked on | planner | model | without the literal | without either | median, planner / model |
|---|---|---|---|---|---|
| corpus0, hogql (341,091 steps) | ×1.93 | ×1.83 | ×1.81 | ×1.84 | ×1.02 / ×1.05 |
| hogql, corpus0 (106,933 steps) | ×1.81 | ×1.64 | ×1.59 | ×1.61 | ×1.02 / ×1.08 |

  It misses less across corpora, both ways, weighted by work; the median is still worse. Between two runs of the
  same pair the model's figure moved by up to ×0.3 (an earlier run: ×2.15 asked on hogql): the forest's draws.
- **The literal does not carry across**, and a new rule's literals are unseen: it is not a feature.
- **Membership.** As an access (every column bound), the planner's median miss is ×2.05 to ×2.15: it assumes a key
  is present, and half the time the step passes under half its rows. The model halves its weighted miss asked on
  corpus0 (×3.68 to ×2.36), not on hogql (×5.05 to ×4.77). With the membership *basis* (a fanout of 1 assumed) it is
  worse than the planner both ways (×1.32 to ×1.61; ×2.17 to ×2.68).
- **The orders not kept are where it is worse**: on the replaced orders' steps ×1.92 to ×2.07 and ×2.13 to ×2.25,
  against better on the orders kept. Trained only on the orders a plan kept, it learns their misses, not those of
  the orders it would choose instead: what step 3 needs. A cold run gives few of them: 1,482 replaced steps of
  341,091, and 8 to 10 trial steps (trials run on a previous run's critical path, decision 0274).

## Labels from runs, and runs the model plans (2026-11-17)

The user: "the training data should be from model runs, not from the planner", and "would it be better to predict
the ratio rather than the absolute value?". Agreed reading: (1) the label is what a run measured, and (2) the data
comes from runs whose orders the model chose, with exploration, not only from the planner's choices.

**The target.** Tried, across corpora (trained on one, asked on the other, weighted miss; planner / model):

| target | features | asked on hogql | asked on corpus0 |
|---|---|---|---|
| measured fanout, ln((out + 1) / (in + 1)) | data only: relation, bound columns and mask, columns, rows, key distinct values, rows per key, solving, seed | ×1.87 / ×2.18 | ×1.82 / ×2.10 |
| measured fanout | the same and the statistics' estimate (the prior) | ×1.90 / ×2.00 | ×1.88 / ×1.88 |
| measured over the statistics' estimate | the same | ×1.86 / ×1.87 | ×1.83 / ×1.83 |
| measured over the statistics' estimate | the first ratio model's (basis, place, rows in, …) | ×1.93 / ×1.83 | ×1.81 / ×1.64 |

The fanout itself loses: a forest predicts piecewise constants, and with the estimate as a feature it still could
not follow it across six orders of magnitude (kind-frequency steps ×14, where the statistics missed by ×4.5). The
ratio to the statistics' estimate is the target: the label still the run's measure, the estimate (a constant's
frequency, a key's postings) what the model corrects. The step's place and its estimated rows in carry most of the
first model's lead; the data features are kept with them (`STEP_FEATURES`).

**Runs the model plans** (`RERAIL_JOIN_MODEL=1`). The cost statistics' read starts a thread that trains the model on
the machine's stored steps (200,000 in 2.4 to 3.8 s; none under 10,000) and sets it as the batch pricer
(`joins::set_pricer`); until then the planner's estimates choose. With a pricer each searched line's candidates
carry their steps' features (`Candidate::steps`, from each order's plan record), and the pricer scales the planner's
price by the rows the model predicts over the statistics' (`join_steps::model_price`: each step's rows the step
before's times its corrected fanout). Of the model's choices, `explore_rate` (0.1) take another candidate within the
margin, so the steps it learns from are not only its picks. Its samples are kept as origin `model`.

One store, in turn (m3, cold, profiled): corpus0 and hogql planned by the planner, hogql planned by the model,
hogql by the planner again.

| hogql | rows of work | strata CPU | load |
|---|---|---|---|
| the planner | 1.412 G | 130.9 s | 22 |
| the model | 1.368 G (−3.1%) | 155.6 s | 39 |
| the planner again | 1.412 G | 153.5 s | 78 |

The CPU is the host's load, not the plans (the planner's two runs differ by 23 s). The model's run made 1,483 choices
other than the search's best, and explored 614. With its own run's 382,585 steps in training, asked on the
planner's last run: weighted ×1.88 to ×1.76, rms ×3.60 to ×2.31, median ×1.02 to ×1.08; membership tests ×4.42 to
×2.10 (median ×2.00 to ×1.41), kind frequency ×5.82 to ×3.81, postings ×2.47 to ×1.60; filters ×1.58 to ×1.70.

Findings: corpus0 planned twice by the planner and twice by the model (other exploration seeds) gives the same 660
relations and 30,449 rows, byte for byte (`rerail-engine csv`).

## The model's planning on the GPU (2026-11-17)

The user: the model's planning must cost much less than the work it saves; what it does on the CPU for its
predictions goes to the GPU. Measured first (hogql, a model-planned run against the planner's, profiled spans):

| on the host | CPU | calls |
|---|---|---|
| prediction: the forest walked per step, a choice at a time | 3.98 s | 133,747 (102,324 of them rounds with nothing to choose) |
| features: each candidate's plan record (`explain_order`, its text read back) | 1.6 s | 33,314 |
| training: the feature space's buckets made on the host, the forest on the device | about 0.5 s of its thread | once |

A dispatch costs the host about 60 µs and 0.24 µs a step (M3, `the_device_predicts_as_the_host_and_its_cost`),
a walk on the host about 9 µs a step: a choice at a time (3 to 15 steps) the device would cost more. So:

1. **One prediction a round, on the device.** The pricer predicts every step of every candidate of a round in one
   call (`JoinModel::fanouts`): on the device from `DEVICE_ROWS` (8) steps, where the device buckets the values and
   walks the forest (`DeviceForest::predict_values`, resident from training), else on the host. Rounds with
   nothing to choose do not ask it.
2. **A stratum's first plans in one round** (`choose_first_plans`): the seeds', then the probes' lines that will be
   searched (no kept plan, at least two) are planned first with a round open (`plan_rule_reused_in`); their
   choices are made after one prediction and kept under their keys (`keep_choice`), which the spec's loop then
   takes. 21,026 predictions a line at a time became a few per stratum.
3. **Fewer features.** A candidate the statistics price over `RERAIL_JOIN_FEATURE_SPAN` (16) times the best's is not
   explained or predicted; the search's best reuses its own record.
4. **Training's buckets on the device** (`from_columns_device`, the host's buckets: the device's predictions match
   the host's on 4,000 steps).
5. **Exploration only where cheap**: the model explores lines it prices under `RERAIL_JOIN_EXPLORE_ROWS` (1 M): an
   explored 59 M-row line (`p299_attempt`) ran 230 M.

hogql, one store, cold, profiled (m3; the host's load 25 to 53):

| | planner | planner | model | model |
|---|---|---|---|---|
| a stratum's planning (`spec`, inclusive) | 4.01 s | 3.84 s | 6.45 s | 3.73 s |
| prediction | | | 0.94 s (13,168) | 0.50 s (10,904) |
| features | | | 0.94 s | 0.67 s |
| the round itself | | | 0.29 s | 0.15 s |
| rows of work | 1.39 G | 1.41 G | 1.44 G | 1.40 G |

The model's planning overhead fell from about 6 s to 1.3 to 2.2 s of CPU. Training: 2.4 s (7.1 s under load),
0.5 s of its thread's CPU. Its work is the planner's within the runs' spread (the first model run's −3.1% was
within it too). corpus0, planner and model twice each: the same 660 relations and 30,449 rows.

What stays on the host: the features (each candidate's plan record: the planner's statistics read per step, not
device work), and a dispatch's 60 µs where rounds are small (the driver's per-wave rounds, live replans).

## Features from the search, and the prediction armed ahead (2026-11-17)

The user: the features (each candidate's plan record) go too, the search's own per-step rows taken instead; and the
GPU's command is submitted before the round is built, so it runs once the round's rows are written.

1. **The search's steps** (`SearchCx::steps_of`): with a pricer set, the search hands back its best's and its
   runners-up's steps from its own cells: each join's rows a row in and the rule that priced them (`Cell::fan`,
   `basis`; a runner-up's last join from its `finals` entry), each filter's share, access and source by kind.
   `decide` takes them before any record; a proposal the search did not price is still explained. A prediction's
   relation and bound columns come from the body, not the record's text (the samples' too, so both agree), and
   `bound_columns` is gone. `the_searchs_own_steps_are_the_records_features`: rows, basis, access and source as the
   record of the same order gives them, with and without a delta.
2. **The prediction armed** (`DeviceForest`'s `Armed`): bucketing and the walk encoded and committed ahead into a
   command buffer that waits on a shared event, on a queue of its own (a waiting command buffer holds back those
   committed after it on its queue). A call writes its values into that command's buffers, signals, waits; the next
   is armed after it, and training arms the first (`prepare_values`, 1,024 rows). Its sizes are fixed when armed
   (the kernels' plans take the rows): a call of fewer rows packs only its own, the rest bucket 0 and unread; a
   larger call, or columns of other kinds, releases it and arms a larger one. Dropping the forest releases it. The
   trees are summed over lanes chosen for the armed rows, so the predictions equal the unarmed ones to rounding
   (`an_armed_prediction_is_the_unarmed_ones`). The GPU's seconds exclude the wait (0.038 ms for a call 20 ms after
   arming). A call's host CPU (M3, `the_device_predicts_as_the_host_and_its_cost`): 61.6 to 38.5 µs at 1 row, 63 to
   47 µs at 10, 86 to 72 µs at 100, arming the next included.

hogql, profiled, inclusive CPU:

| | before (M3) | | after (M3) | | after (M1 Max) | |
|---|---|---|---|---|---|---|
| features | 0.94 s | 0.67 s | 0.13 s | 0.10 s | 0.07 s | 0.13 s |
| candidates explained for them | 25,341 | 22,721 | 177 | 122 | 15 | 53 |
| prediction | 0.94 s | 0.50 s | 0.90 s | 0.71 s | 0.57 s | 0.87 s |
| of it on the device / on the host (calls) | | | | | 0.29 s (3,147) / 0.26 s (3,236) | 0.46 s (6,254) / 0.35 s (5,834) |
| a stratum's first plans in one round | 0.29 s | 0.15 s | 0.16 s | 0.18 s | 0.04 s | 0.17 s |

Work on the M1 Max: the planner 1.37 G and 1.41 G, the model 1.39 G and 1.41 G. Findings, hogql planned twice and
by the model twice: the same 660 relations, 100,617 rows (each relation's rows sorted; the files' own order follows
the join order). The prediction is now most of what is left: about 80 µs a round either way, a round of a few
steps on the host, a larger one on the device; the per-wave rounds are small.

Found on the way: a plan record is made only when the run is profiled, binds the device, or reads a kind-set axis.
A CPU-bound unprofiled run therefore stored no samples (and, before step 1, priced no candidate). The search's
steps are not handed to the record's other readers (placement's `add_joins` reads the columns' names and the
walked rows they lack), so that gap stays open (Q82).

## Step 2, and a stratum's plans in one round (2026-11-17)

The user asked for the planning cost down further. Ranked by own CPU in the hogql profiles, four candidates; two
were misread:

1. **Exact pair statistics (step 2)**: with the model pricing, a slow line's exact pair statistics are not counted
   (`exact_statistics`'s callers): the flag, the forced replan (priced by the model, in its round) and the wider
   search's request stay. They were 3.2 to 4.4 s of CPU a hogql run, 29 to 42 counts, one stratum's 1.8 s.
2. *The model doubled the searches*: no. A stratum's spec runs once a chunk of guest files (about 2,000 a chunk),
   and the chunks vary from run to run for the planner too (34 to 63 on hogql). `keep_choice` prices one order
   (no search).
3. **A stratum's plans in one round** (`plan_ahead`, was `choose_first_plans`): every line of the stratum, not
   only those seen first, is planned before its loop with the round open, and its plan handed to the loop
   (`Ahead`, by `line_id`), which plans only a line missing from it. Lines the loop searched one at a time were two
   in three of the prediction rounds; the whole plans' replans are one round a chunk, as the probes' were. The
   device takes a round from 6 steps (`DEVICE_ROWS`, from the armed call's 38 µs and 0.9 µs a step against the
   host's 8.3 µs a step). Rounds now: one a stratum before its loop (6,011), one a chunk of live replans (3,116);
   fewer would merge rounds of strata running at once.
4. *Probe re-planning*: no. `probe: order of a derived input` (3.3 to 3.7 s) is the sorted index a derived input
   gets when first probed (decision 0112), execution. Probe and whole replans' searches were 0.4 s in all.

hogql, the M1 Max (load 12 to 17), one store:

| | planner | model | planner | model |
|---|---|---|---|---|
| chunks | 39 | 36 | 33 | 38 |
| a stratum's planning (`spec`) | 5.31 s | 5.16 s | 3.74 s | 5.27 s |
| exact pair statistics | 3.88 s | 0 | 4.45 s | 0 |
| prediction (on the device / host, calls) | | 0.85 s (5,462 / 3,642) | | 0.78 s (5,952 / 4,185) |
| strata CPU | 184.0 s | 161.9 s | 180.1 s | 163.7 s |
| rows of work | 1.42 G | 1.35 G | 1.46 G | 1.39 G |

The model's planning, statistics included, is 5.2 to 5.3 s against the planner's 8.2 to 9.2 s; its work 4.8 to
4.9% under the planner's in both pairs (earlier pairs were within 3%, so two more pairs would settle it). Findings,
hogql planned twice by each: the same 660 relations and 100,617 rows.

## Rounds shared across strata, buckets as numbers, the device's threshold measured (2026-11-17)

The planning threads plan strata at once, each asking its own round. Three changes:

1. **Rounds share a dispatch** (`JoinModel::fanouts`). A round queues its rows. If no dispatch is under way, its
   thread makes one of every round queued. Otherwise it sleeps on its own slot, until a dispatch serves it or hands it
   the next one (it is first in the queue when one ends). Each round is woken once. A dispatch that fails or panics
   serves every round an error, so the planner's estimates choose. On the M3, 16 threads asking 800 rounds at once
   made 62 to 100 dispatches. A round cost 20 µs of process CPU, against 32 µs asked from one thread. A first
   version, with one condition variable woken for every round served, cost 57 µs.
2. **Buckets as numbers** (`DeviceForest::predict_columns_timed`, `PredictColumn::Buckets`). The categorical
   columns (relation, access, source, basis) are bucketed on the host, by a hash table built at training. They reach
   the device as bucket numbers through identity tables, not as strings bucketed there. The strings' bucketing was
   most of a call's host side.
3. **The threshold is measured per forest** (`JoinModel::calibrate`). At training, the thread's CPU for a host walk
   of up to 64 training steps is set against an armed dispatch of the same steps, the best of four. The tests' small
   forest walks a step in 1.8 µs against a 15 µs dispatch, so the device takes 9 steps or more. The engine's forest
   (200,000 steps) walks a step in 7 to 22 µs, so the device takes every round of 1 to 3 steps or more.

**A correction.** The per-call numbers in the sections above, and the first threshold (6 steps), came from the
tests' build. That build is the dev profile, which compiles workspace crates unoptimized and walked the forest 5
times slower. Measured in the `profiling` profile on the M3:

| rows | host walk | device (armed) |
|---|---|---|
| 1 | 0.6 µs | 46.9 µs |
| 10 | 13 µs | 50.0 µs |
| 100 | 185 µs | 56.8 µs |
| 1,000 | 1,880 µs | 142.5 µs |

These are the small test forest's walks. The run-level numbers in earlier sections were from optimized runs and
stand.

hogql, on the M3 under load 22 to 92, one store. The first pair has 1 and 2; the second pair adds 3:

| | planner | model | planner | model |
|---|---|---|---|---|
| chunks | 39 | 40 | 36 | 40 |
| a stratum's planning (`spec`) | 3.56 s | 5.66 s | 4.37 s | 5.71 s |
| exact pair statistics | 2.82 s | 0 | 3.02 s | 0 |
| prediction span (calls) | | 0.94 s (10,833) | | 0.69 s (11,940) |
| the model's CPU: bucketing, waiting, dispatching or walking | | 0.026, 0.023, 0.648 s | | 0.026, 0.021, 0.385 s |
| dispatches / host walks, of 8,192 rounds | | 1,548 / 5,032 | | 5,408 / 1,842 |
| strata CPU | 141.9 s | 144.7 s | 156.9 s | 153.3 s |

With exact statistics counted, the model's planning is 5.7 s against the planner's 6.4 to 7.4 s. The strata CPU
moves with the load by more than the planning does. Findings, hogql planned twice by each: the same 660 relations
and 100,617 rows. The out files' hash in the comparison script differs between two planner runs too (file order),
so it is not a findings check.

**A stratum planned once per run (item 4, not done).** Each chunk plans every stratum, but its lines' plans are
already kept. Of the model run's lines, 55,140 were kept at their price, 40,378 kept and re-priced, and about 35,000
taken under the budget, against 16,151 searched. What a chunk repeats is the check that a kept plan still holds, and
the spec built from it. Per line, that is a key (1.2 µs), its sizes (1.7 µs), the budget (0.6 µs) and the join
spec (4 µs). A stratum's own part is about 19 µs. In all it is 3.6 s (planner) to 5.7 s (model), of 140 to 155 s of
strata CPU. Keeping a stratum's spec whole while its inputs barely change cannot skip those checks: 30% of lines'
sizes move more than 2× between chunks, and the budget, the mispricing check and pending work read what the last
chunk's run measured. Only the join specs and the stratum's own part could be kept, about 2 s at most. Planning
once per run and taking the plans as they are would save all of it, but would stop a chunk's plans from following
its sizes. That choice is the user's.

## Not done

- **Samples of orders not kept, made on purpose**: an offline generator running each line's other orders over the
  captured components (the lab loop), so the model sees the orders it would pick, not only those picked.
- **The step's first rows.** A probe's delta and a seed's anchor (step 0) are not samples: their estimates are a
  relation's size, not a fanout. The largest stratum misses on hogql are there (a recursive stratum's delta, the
  rounds of a closure), and stay the planner's.
- **The volume.** One run wrote 336,397 steps (60 MB of store). Steps of one line at one place repeat across
  components: they could be one sample with counts.
- **Membership** is worse corrected (×1.06 to ×1.40): the next comparison reports it apart.
- Steps 2 and 3.
