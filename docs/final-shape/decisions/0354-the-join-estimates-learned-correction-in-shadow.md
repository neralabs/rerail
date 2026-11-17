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
