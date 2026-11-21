# Deferred join lab

The lab compares physical plans for the relation `wide(a,b) = exists k: L(a,k) and R(b,k)`.
A reader can bind either column, bind a pair, or request every distinct pair.
Each run uses the same inputs, reader requests, and set semantics.

## Run the small cases

From the repository root, run these commands with Python 3.9 or later and SQLite 3.35 or later.

```sh
lab=scripts/profiling/deferred_joins
mkdir -p tmp/join-tests
nice -n 19 python3 "$lab/lab.py" --out tmp/join-tests/small.json
python3 "$lab/report.py" tmp/join-tests/small.json
nice -n 19 python3 "$lab/test_lab.py"
```

Use a new report name for each experiment.
Select cases with `--cases` and plans with `--modes`.
Use `--cache-keys-rows 0` to disable retention of tabled answers.
Set `--repeats` to change the number of samples. The default is three.

| Case | Question |
|---|---|
| `empty` | How much work starts before a reachable reader supplies its first key? |
| `cartesian` | Can sparse pair requests avoid a product with one common join key? |
| `sparse` | Do correlated requests keep only their own pairs across several join keys? |
| `hot-key` | How does a popular key affect membership and one-sided reads? |
| `mixed-readers` | Can readers bind opposite columns of one producer and share its factors? |
| `full` | What does a complete enumeration cost? |
| `discovered` | Can one round use earlier answers to request new keys, then repeat those requests? |
| `rounds-edits` | Do a second witness, a deleted last witness, and an insertion after a cached miss give correct answers? |

## Run PostHog

Use a local PostHog Git clone containing commit `37c895112cd9830ba6161037c34c431a2a6d7d0c`.
Set `engine` to an optimized Rerail CLI and `posthog` to that clone.
The historical sample contains 1,644 files. The 1,664-file sample extends it by 20 files.

```sh
lab=scripts/profiling/deferred_joins
for count in 1644 1664; do
  nice -n 19 python3 "$lab/corpus.py" --engine "$engine" --git "$posthog" \
    --count "$count" --out "tmp/join-tests/posthog-$count"
done
nice -n 19 python3 "$lab/lab.py" --cases --coverage \
  --capture tmp/join-tests/posthog-1644/case.sqlite \
  --capture tmp/join-tests/posthog-1664/case.sqlite \
  --out tmp/join-tests/posthog.json
python3 "$lab/report.py" tmp/join-tests/posthog.json
```

The sampler ranks Python paths under `posthog/` by SHA-256. It reads the selected blobs from the pinned Git tree.
The runner checks the corpus digest, then runs the original SLF001 policy cold, with capture, and warm.
Each run must reproduce the saved digest of **every diagnostic field**.
The runner keeps the reports, capture, and manifest. It removes its temporary corpus and stores.

The native relational lab captures the class-name branch of `slf_class_head` at `slf_class_head__sup2`.
The adapter checks the rule body, variable positions, constants, and projected head before it accepts the capture.
The left factor relates an identifier node to its text. The right factor relates an attribute access to an enclosing class name.
Their common name supplies the join key. The native reader supplies correlated node pairs.
Replay checks the matching pairs against the projection of the native output table.
With `--coverage`, a separate stress case asks for all matches of each requested left node, then every pair.
A full relational query supplies the reference for these additional requests.

Use the original policy to check a candidate engine.

```sh
nice -n 19 python3 "$lab/corpus.py" --verify-only --check-edit \
  --engine "$candidate" --git "$posthog" --count 1644 --out tmp/join-tests/candidate
```

This command also works after a planner change removes the captured helper.
The edit adds a source file to the temporary sample and compares the updated store with a fresh run.
Removing the file must restore the original diagnostic digest.

## Physical plans

| Mode | Work and state |
|---|---|
| `eager` | Build and index all distinct pairs, then answer the reader. |
| `direct` | Probe the indexed inputs for the caller's bindings. This control isolates the effect of caching and extra demand rules. |
| `demand` | For pair reads, batch the distinct bound columns and restrict each factor before the join. Test the original pairs against those factors. One-sided requests bind the relevant input directly. |
| `factors` | Keep the two derived factors and index each in both directions. Reuse existing indexed factors on synthetic cases. Join on each read. |
| `reduced` | Intersect the join keys first. Build and index only the factor rows that have a key on both sides. |
| `tabled` | Batch new requests. Cache complete answers and misses. The binding pattern and values identify an entry. |

The tabled plan charges one budget unit per key and per retained answer row.
It evicts the least recently used entries and skips retention for an oversized answer.
An edit invalidates all cached answers and derived factors. This tests conservative invalidation.
The spike covers finite, positive joins with set projection. Recursive completion, concurrent readers, and persistent partial results need separate experiments.

## Read the measurements

Each strategy runs alone in a fresh process. Repetitions rotate the strategy order.
The worker measures CPU and elapsed time for edits, state construction, queries, and answer collection.
It measures checksum and sorting time separately. Peak RSS covers the entire worker.
The query counter estimates SQLite virtual-machine steps in increments of 1,000.
The row counters report explicit factor and result writes. SQLite can also use internal temporary tables.
Common capture preparation and base indexes precede the timed operator.

The PostHog manifest records whole-policy native times separately from SQLite operator times.
Use the native baseline to judge an engine integration. Use replay to compare competing join strategies.
The worker uses a 45-second SQL deadline and a 256 MiB SQLite heap limit.
The runner skips eager builds and full-coverage stress cases above the witness budget.
Native runs use four threads, nice 19, a 120-second deadline, and a sampled 2,250 MiB RSS limit.

## Reference run

The historical corpus produces 490 diagnostics. The larger corpus produces 491.
Each class-name capture contains 228 correlated requests, one matching pair, and 227 misses.

On 1,664 files, direct probes use a median 2.35 ms of CPU for the original reader.
The tabled plan uses 2.29 ms cold and 0.041 ms on repeat.
Building all pairs uses 1,416 ms. Building reduced factors uses 335 ms and writes 38,319 factor rows.

The wider reader changes the choice. It requests 167 left nodes, repeats that request, then enumerates all 297,144 pairs.
Reduced factors use a median 601 ms across the three phases, including construction.
Direct probes use 1,805 ms. Eager materialization uses 1,534 ms.

[reference.tsv](reference.tsv) contains the phase medians for three repetitions per strategy.
The reduced-factor experiment ran in a separate series on the same machine.
Both series used Python 3.15.0 and SQLite 3.54.1.
The native control used the saved profiling binary with the local planner changes, before the segment-writer fix.
Its SHA-256 is `3e82a44d42820f52fc11d4b7008ec432c1f07b543ed3b738170d0485d996a150`.
The CLI runs the native policy. The three experimental strategies run in SQLite replay.

## Add an experiment

Add a reader schedule to `cases.py` to test a new access pattern.
Add a mode to `strategies.py` and `MODES` to compare another algorithm.
Each mode returns rows grouped by the original request, including empty answers.
The runner checks each phase against its reference and stops on a mismatch.

For a new captured producer, expose four SQLite tables or views.
Use `l(a,k)` and `r(b,k)` for the factors.
Use `requests(a,b)` for the actual pairs and `expected(a,b)` for the native matching pairs.
Pass the database to `lab.py --capture`.
The PostHog adapter illustrates how to check the captured rule before interpreting its columns.

## Native planner experiment

The native corpus planner recognizes a binary relation whose two sides meet at a hidden join key.
For example, `wide(a,b)` can mean that `left(a,k)` and `right(b,k)` agree on some `k`.
Each side can contain several joins. The policy author keeps the original rule.

| Readers | Planner action |
|---|---|
| Every reader supplies a correlated pair | Collect the distinct requested pairs. Drive the producer from that relation and keep its answers. |
| All readers independently bind the same column | Collect those endpoints. Compute the producer only for the requested endpoints. |
| Positive readers need different columns or a full scan | Keep each factor with the hidden key. Reduce it by the other side's keys. Join the factors in each reader. |

The pair table also identifies completed misses. After the producer's stratum completes, a requested pair absent from its answers has no witness.
The ordinary relation and index machinery handles repeated reads. It also maintains the new rules when inputs change.
A broad reader can still enumerate every pair. If it projects a smaller output, it can discard the joined pair inside its own rule.

`RERAIL_DEFERRED_JOINS=on` selects the static rewrites. The default mode, `auto`, also shares work for repeated requests and admits small pair caches.
`adaptive` selects the same behavior as `auto`. Set `RERAIL_DEFERRED_JOINS=off` for the control.
The experiment picks the more complex side by literal count. If that side contains one literal, it uses the original indexed read.
For each endpoint, it counts the distinct pairs that request it. Below eight, it probes directly.
At eight or more, it derives the endpoint's distinct join keys once. Each request joins those keys to the other side.
Both paths retain the original request tuple. The threshold is an experiment parameter built into the planner.
The preparation cache includes the mode in its key.

The pass uses the existing demand proof and shared-key factorization. It adds no tape format, block layout, device upload, or executor-specific copy.
The new state consists of rule heads and their normal indexes. The cost moves to additional strata, demand collection, and factor maintenance.
Storage for pair demand grows with distinct requests. A hot endpoint with many witnesses can still produce a large factor.
The fanout test measures repeated requests. It cannot tell how cheap the original index probe is or how many witnesses an endpoint has.

The general rewrite requires one definition, two distinct projected variables, and a nonrecursive producer.
A rule already guarded by tuple demand can share a factor even when its head has other alternatives.
The pass preserves aggregate readers, open negation, native consumers, and explicit planning constraints through its eligibility checks.
It requires a proof of independent bindings before it restricts a relation. An unbound positive reader can require both factors to be built even when a later filter returns no rows.

Run the native cases and a pinned corpus from the repository root.

```sh
lab=scripts/profiling/deferred_joins
nice -n 19 python3 "$lab/native.py" --engine "$engine" --out tmp/join-tests/native.jsonl
nice -n 19 python3 "$lab/corpus.py" --verify-only --check-edit --deferred adaptive \
  --engine "$engine" --git "$posthog" --count 1664 --out tmp/join-tests/posthog-adaptive
```

The native cases check complete outputs against independent set calculations.
They also read the engine's profile to check which rewrite ran. The corpus check uses the original policy and the pinned diagnostic digest.

The `edits` case reuses one store and the same rule text across seven runs.
Adding a source raises one endpoint from seven to eight requests. Two other sources provide independent witnesses for a previous miss.
Deleting the first witness preserves the answer. Deleting the last witness restores the miss.
Deleting the request source puts the endpoint below the sharing threshold again.

This case also exercises ownership of retained input rows. A query can use a document's path and omit its ID.
A retained input then reads the unused owner column as zero. On an edit, the engine cannot identify a file's rows using that value.
It rebinds the inputs from the memo when changed rows lack their owner column.
An input with no changed rows can keep its state. The report records when the engine takes the fallback.

### Cost and scope

Let `D` be the set of requested pairs. Let `L[k]` and `R[k]` be the rows on each side with join key `k`.
An eager join can visit `sum(len(L[k]) * len(R[k]))` witnesses before duplicate elimination.

Pair demand stores `len(D)` requests and at most `len(D)` answers. The probe cost still depends on the indexes and witnesses for each pair.
The adaptive branch also stores one row per hot endpoint and one per distinct witness key for that endpoint.
Reduced factors store the supported rows of each side and each side's distinct keys. Enumerating a full answer still costs at least its output size.
These bounds describe the logical relations. The executor also keeps indexes and may retain support rows for maintenance.

The planner indexes definitions and reads in one pass. The demand proof traverses dependencies and checks stratification for each selected producer.
With many candidates, that repeated work can grow as the number of candidates times the program size.
The broad rewrite also runs functional-dependency analysis. The existing factor pass limits reader expansion.
The first eligible hidden key and the number of literals are structural heuristics. They do not measure probe cost, key skew, or the size of a factor.

### Native verification

The native suite checked 48 case, mode, and edit combinations. The factor test also exposed the generated hot set and factor as outputs.
At the threshold, one endpoint held two distinct witness keys. Each mode passed all seven successive store edits.

Both larger synthetic cases have 57,360 distinct possible pairs. The table sums `heads.tsv` work for the producer, helpers, and output readers.
The engine counts rows entering join steps plus rows leaving each plan's final step.

| Native case | Output | Off | Static demand or factors | Adaptive |
|---|---|---:|---:|---:|
| 480 correlated pair requests | 240 hits and 240 misses | 124,533 | 5,108 | 6,049 |
| Two readers bind opposite columns and project existence | 3 left and 2 right endpoints | 125,013 | 7,838 | 7,838 |

The pair case favors direct demand even though two endpoints each have 240 requests.
The second case uses the same reduced factors in both experimental modes.

The original PostHog policy reproduced every diagnostic field on 1,644 and 1,664 files.
In adaptive mode, a source insertion and deletion gave 490 → 491 → 490 and 491 → 492 → 491 diagnostics, respectively.
Each inserted snapshot also matched a fresh cold store.

On 1,664 files, two cold samples per mode used the same profiling binary and fresh stores. The second series reversed the mode order.
The one-minute system load ranged from 35.6 to 58.4 across these runs. Each run used four engine threads.

| Mode | Cold wall seconds | Cold CPU seconds | Cold peak RSS, MiB |
|---|---:|---:|---:|
| Off | 7.47, 10.47 | 17.42, 20.56 | 594, 601 |
| On | 9.44, 8.36 | 18.87, 17.19 | 634, 587 |
| Adaptive | 7.85, 8.06 | 18.09, 16.70 | 593, 606 |

The generated class-head plan retains correlated pair demand on every branch. The adaptive branch selects the identifier as its endpoint and the class node as its witness key.
In the saved 228-pair capture, one identifier reaches eight requests. All eight miss. The other 220 requests use direct probes at this threshold.
The experiment shows a reduction in join work on the synthetic cases. The two whole-policy samples per mode give overlapping CPU ranges on PostHog.
The experimental modes require explicit selection. A stronger promotion rule needs the observed cost of a direct probe and the estimated number of witness rows it will retain.

The profiling binary's SHA-256 was `7ec82bb7dd0e96729ea5d2731485935698c5737776a359e4c22b77ff8c163e81`.
Use `--check-plan` with the corpus runner to save its generated cross-file rules and verify the class-head rewrite.

## Native cost on SLF001 and TRY401

The cold SLF001 profile used 1,664 files, four threads, and the original policy. It reproduced all 491 diagnostics.
The engine measured CPU by rule. The macOS sampler also captured stacks at 10 ms intervals.
The sampler observes waiting threads. Use the engine counters for CPU attribution.

| SLF001 cost | Evidence from the native run |
|---|---|
| Immediate request rules | 6.9 ms CPU across the class-head demand and support rules. They request 228 pairs. |
| Answer rules | 66.6 ms CPU. The class-name branch tests 176 enclosing-class lookups and returns one pair. |
| First enclosing-class lookup | The engine spends 51.9 ms sorting a stored column with 1,327,192 rows. |
| Text lookup in that branch | The text step spends 14.0 ms testing 122 rows and returns one. |
| Shared inputs | The request rules read `slf_ann_head` and `attr_of`. Other rules also read these relations. |
| Projection | The full projection and interning phases use 3.67 s CPU. They visit 3,894,406 scalar nodes. |
| Name lookup | `wstart` uses 1.06 s CPU. Several readers repeat its identifier, field, and text checks. |
| Recursive qualified names | One plan joins 10,981 scope rows to 1,110,702 candidates. Its definition filter retains 2,532. This plan uses 199 ms CPU. |

The process used 23.01 s CPU with profiling. Direct rule costs exclude their shared inputs.
An earlier text-selector audit on 1,644 files rejected only 1,331 of 1,343,714 eligible nodes.
Projection includes structural work and other scalar relations. Its full CPU cost overstates the potential saving from narrower text selectors.

### Test repeated name lookup

The planner inlines `nameref` in the control. The experiment stores it once to share the identifier checks and name across readers.
The runner changes one storage hint in a temporary library. It checks the physical plan and every diagnostic.

| SLF001 with 1,664 files | Default | Store `nameref` |
|---|---:|---:|
| All native join work units in the final profile | 34,887,240 | 29,406,976 |
| Work units in `wstart` | 6,721,130 | 2,694,447 |
| Work units to produce `nameref` separately | 0 | 2,564,679 |
| Scalar node visits | 3,894,406 | 3,894,406 |
| Cold CPU seconds | 21.48, 22.05 | 20.56, 20.06 |
| Cold wall seconds | 20.65, 16.20 | 22.97, 21.64 |
| Cold peak RSS in MiB | 583, 609 | 588, 595 |

Each pair lists results in time order within that mode. The four cold runs used default, stored, stored, default.
The one-minute load ranged from 50 to 69. Both comparisons favor storage for CPU. Elapsed time increased in both.
The physical reduction survives the cost of computing the shared relation.
`wstart` and the stored producer together take 1.28 s of join CPU. The default `wstart` takes 1.06 s.
Other readers contribute the saving. The planner estimated the cost on both sides of each original inlining decision.
A generic inlining decision needs the repeated cost across all readers, including negative field checks and text reads.

### Test reduced factors on a second policy

The source survey tested eight unmodified Python policies on 100 files. TRY401 rewrote `exc_qual(expression, qualified_name)` into two factors.
Its readers look up names from call sites and find logging calls by qualified name.
The 600-file sample uses the first 600 paths from the pinned ranking. It contains one TRY401 finding.

| TRY401 with 600 files | Default | Static rewrites |
|---|---:|---:|
| Rows from the original join or each reduced factor | 65,830 | 65,830 on each side |
| Producer and helper work units | 197,493 | 1,310,171 |
| Producer and helper CPU | 45.9 ms | 133.6 ms |
| Cold policy CPU seconds | 17.30, 14.48 | 16.90, 15.23 |
| Cold peak RSS in MiB | 912, 909 | 937, 954 |

The largest helper uses 783,523 work units to establish the keys on one side.
The extra writes and probes outweigh the work saved by deferring this join.
TRY401 also triggers other rewrites. The producer counters measure `exc_qual` and its helpers. Whole-policy times include every rewrite.
In each mode, inserting a logging call raises the count to two. Deleting the source restores the original finding.
Both updated stores match the fresh edited store in every diagnostic field.
The 1,664-file cold trials agree on two findings. One process briefly reached 2,261 MiB, so the detailed comparison uses 600 files.
The runner now checks the operating system's peak after exit as well as sampled RSS during a run.

This trial measures the cost of constructing both factors. Its default column uses the engine before automatic factorization.
Test shared name lookup with an automatic inlining decision before applying it to other policies. Preserve the original source rules.

Use a fresh output directory for each command. The optional stack sample requires macOS.

```sh
lab=scripts/profiling/deferred_joins
nice -n 19 python3 "$lab/corpus.py" --git "$posthog" --engine "$engine" --count 1664 \
  --verify-only --check-plan --profile --sample --out tmp/join-tests/slf001-profile
nice -n 19 python3 "$lab/compare_try401.py" --git "$posthog" --engine "$engine" --out tmp/join-tests/try401
nice -n 19 python3 "$lab/compare_nameref.py" --git "$posthog" --engine "$engine" --out tmp/join-tests/nameref
```

## Limits in the native engine

For eligible binary joins, the planner keeps paired requests together. For mixed positive readers, it replaces the wide relation with two factors.
If multiple readers can reuse a pair cache, the engine counts both factors on the current snapshot.
Each cache gets an equal share of `RERAIL_JOIN_CACHE_ROWS`. The default allowance is 4,096 pair witnesses per prepared program.
If the product of the counts exceeds that share, readers join the factors directly.
For a cache that fits, the join pricer can choose either representation from the counts, reuse and allowance.
The default policy accepts a cache that fits and has at least two readers.
Set the allowance to zero to use factors for every mixed reader.

Some attributes require the source document. The planner checks their placement before it reads corpus rows.
If a shared cache moves a lookup into the cross-file phase, the planner first tries direct factor reads.
If a direct reader still needs the document, the planner computes the attribute in that document before the join.
The profile records the attribute under `deferred join placement`. The prepared program retains the valid plan for later runs.

For a positive scan and single-column probe, the native walker can build a 2 KiB Bloom filter over the smaller input.
When both columns have sketches, the walker estimates distinct-key overlap from their HyperLogLog union.
The walker samples 64 probe rows to account for repeated keys.
If at least half fail the membership test, the default policy selects the filter.
The join pricer can also use the sample and the distinct-key overlap to choose a lookup.
Each positive result still requires an exact lookup.
All workers share a 4 MiB allowance for filter bits and entries in the failed-probe cache.
When a reservation fails, the walker continues with exact lookups.

The row allowance covers optional pair caches. The byte allowance covers the two native lookup optimizations.
Factor relations, indexes, maintenance state and required outputs need additional memory.

Run the threshold test with one shared key. An insert changes the factor sizes from 64 × 64 to 65 × 64.
A delete restores the original sizes. The runner checks every output after each edit.

```sh
mkdir -p tmp/join-tests
nice -n 19 python3 scripts/profiling/deferred_joins/native.py --engine "$engine" \
  --cases cache-edits fanout-mixed --modes auto --cache-rows 4096 --out tmp/join-tests/bounded.jsonl
```

### Compare the shipped Python pack

Use the prepared `samples-python` directory and its shipping manifest.
The runner checks all 2,313 paths and file contents before it starts the engine.
It checks every shipped rule with `check-rr` and records each rejection in `comparison.json`.
Both modes run the same accepted rules in order, each with a fresh store.
The comparison includes every diagnostic field and preserves duplicate findings.

```sh
nice -n 19 python3 scripts/profiling/deferred_joins/shipping_pack.py \
  --engine "$engine" --manifest "$manifest" --corpus "$samples_python" \
  --out tmp/join-tests/shipping --rss-mib 8192 --seconds 900
```

Each run records CPU time, elapsed time, load, and peak memory.
The runner stops a run at its memory or time limit, or when free disk space falls below 8 GiB.
If the engine cannot run a group together, the runner saves its error and stops that mode.
A stopped run leaves the comparison incomplete.

### Corpus results with the automatic planner

Each cold trial used a fresh store, the same profiling binary, four engine threads and a 2,250 MiB stop limit.
The control used `RERAIL_DEFERRED_JOINS=off`. Both modes used the same native lookup code.
The table gives the range of two trials per mode for the whole policy. The second comparison reversed the order.

| PostHog sample | Findings in each mode | Control CPU | Automatic CPU | Control peak RSS | Automatic peak RSS |
|---|---:|---:|---:|---:|---:|
| SLF001, 1,664 files | 491 | 16.90–19.41 s | 11.16–20.56 s | 612–645 MiB | 604–617 MiB |
| TRY401, 600 files | 1 | 14.96–15.63 s | 13.96–15.11 s | 892–898 MiB | 922–941 MiB |

Both modes matched the reference digest for every diagnostic field.
SLF001 kept the correlated request on each class-head rule. Its warm run took 1.62 s elapsed and 1.63 CPU seconds.
An insert raised the count to 492. A fresh store matched every field. A delete restored the original digest.
TRY401 kept the two factors for `exc_qual` and joined them in its readers.

## Design for a limit on all intermediate memory

The proposed guarantee covers memory used for intermediate results across concurrent joins in one run.
A second allowance limits the work spent trying a plan that the executor might abandon.
Complete enumeration still needs time proportional to the results a reader requests.
The design below extends the current limits to all intermediate storage and speculative work.

### Which plans to admit

| Reader and input | Candidate |
|---|---|
| No reachable reader requests a row | Skip the producer. |
| All readers supply exact pairs | Probe the original requested pairs. Stop at the first witness for an existence check. |
| Readers supply one endpoint | Probe the smaller indexed side compatible with the request. Consider sharing expensive repeated probes. |
| Readers bind different columns | Keep both sides keyed by their common value. Push each reader's restrictions into its join. |
| A small eager result fits and readers reuse it | Allow eager materialization. A declared unique key can prove that a join grows at most linearly. |
| A reader needs every pair | Stream or partition the join. Include output deduplication in the memory accounting. |

Start with positive, nonrecursive joins. Continue to apply the existing semantic eligibility checks.
For cached existence checks, an edit can remove the last witness. Recompute affected answers or account for their support records.

### What the statistics establish

For each shared key, multiply its left row count by its right row count.
Their sum counts matching row combinations before projection and deduplication.
For sparse readers, also count distinct requested pairs. The unfiltered sum and the number of requested pairs answer different questions.

The lab audit checks a bound on the unfiltered sum. It uses the top keys from either side and their exact counts on both sides.
For the remaining keys, it uses exact row totals, maximum degree bounds, and sums of squared degrees.
Let `L` and `R` denote residual row totals. Let `ML` and `MR` bound the largest residual degrees.
Let `SL` and `SR` denote the residual sums of squares.
The residual contribution is between zero and `min(L*MR, R*ML, floor(sqrt(SL*SR)))`.
This follows from the degree maxima and the Cauchy–Schwarz inequality.

On the 1,664-file PostHog capture, the row totals alone give an upper bound of 20,772,411,712.
The 64 heaviest keys per side give an interval of 170,489 to 6,018,458. The exact total is 297,144.
The sparse reader requests 228 pairs. A direct plan can keep at most 228 distinct positive pair answers.
Demand storage, indexes, and incremental support require separate accounting.

The audit computes both sets of degree counts from the saved SQLite views once. It never enumerates the joined pairs.
The exact total checks the interval and stays out of its calculation.
In the planner, use only certified counts from the same input revision for a hard bound.
A stale count, a sketch, or a missing opposite-side count can inform a prediction. An unmeasured frequency can exceed zero.
Use checked integer arithmetic or outward rounding for certified bounds. The current statistics table also contains floating-point estimates.

```sh
nice -n 19 python3 scripts/profiling/deferred_joins/witness_bounds.py \
  tmp/deferred-join-lab/posthog-1664/case.sqlite --top 64
```

### Where the classifier belongs

The current pass chooses one representation during program preparation.
Keep the legal alternatives in the prepared plan. Score them at execution when the current input statistics and demand become available.
Include the selected representation in keys for any incompatible retained state.

Give each candidate its request pattern, distinct requests, aligned heavy-key counts, input sizes, and index availability.
Include estimated index construction, reuse, and the bytes required by support rows.
Predict per-plan work and peak intermediate bytes. Start with the analytic estimates where training data is sparse.
The batch pricer currently accepts join orders. The existing forest predicts CPU and wall time.
Representation alternatives and memory labels need their own integration with that infrastructure.

Allow an eager plan only with a proven fit or an enforceable reservation and a bounded fallback.
Rank the admitted plans by predicted cost, including the cost of switching. An uncertain prediction can favor the conservative plan.
Give all active workers a common byte allowance for intermediate blocks, indexes, support rows, and scratch.
Reserve capacity before allocation, including space to switch plans. Apply the same rule to factor construction and answer caches.
If an eager build exhausts its allowance, discard the unpublished work and process the same snapshot with a bounded plan.
Allow at most one such switch per choice. Partition or stream larger factors and deduplication state under the same allowance.
A required result that exceeds a resource limit needs explicit exhaustion or bounded external storage. Partial answers cannot establish absence.

### How to judge the decision

Record the features available at choice time, the candidates, the prediction, and the selected plan.
Measure peak charged bytes, work, and any fallback separately. Mark stopped trials as having exceeded their allowance.
Run safe alternatives on the same small snapshots to obtain comparison labels. Keep complete repositories or rule families apart for training and evaluation.
Score maximum intermediate bytes and worst work against the cheapest admissible plan first. Also report tail and median CPU.

Test aligned and disjoint heavy keys with identical row counts, a new hot key after an edit, and duplicate witnesses for one answer.
Force the classifier to choose the wrong plan under a tiny allowance.
The executor must stay within the limit and return the same answers as a fresh run.
Start with shadow predictions on TRY401, sparse SLF001, and the synthetic Cartesian case.
