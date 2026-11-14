# t39: Where an edit's work goes, inside the joins, and in the cold phases (measured 2026-11-14)

## Intro

These measurements set up the next work after decisions 0205 and 0206. They answer three questions on the O125
replay (20 historical commits, 84 files, nera-data-layer):

1. Is an edit's work proportional to the edit?
2. What does the join loop spend its time on?
3. What are the cold run's phases combine, prepare and wait made of?

Every run used a `--profile profiling` build at commit cbc9adbfb plus the "statkeep churn" trace, with
RERAIL_SQL_STRICT=1. Each run ended "same as full". Load averaged about 70.

## 1. Edit proportionality

**Cross part, instructions by files edited (G):**

| step | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19 | 20 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| files | 2 | 1 | 3 | 1 | 14 | 2 | 6 | 6 | 2 | 10 | 2 | 9 | 2 | 1 | 1 | 2 | 2 | 8 | 4 | 2 |
| cross G | 5.33 | 3.77 | 6.10 | 3.23 | 10.55 | 3.73 | 6.15 | 7.36 | 4.75 | 9.09 | 3.65 | 11.12 | 6.65 | 7.07 | 4.77 | 7.19 | 3.28 | 7.69 | 4.76 | 5.30 |

The cold cross part is 8.9 G, so an edit costs 36% (step 04, 1 file) to 125% (step 12, 9 files) of a cold run, and a 1-file edit up to 80% (step 14). Each step runs 273–319 of 338 strata and
maintains 157–265 of them. No stratum is derived again in any step.

The work splits into three causes.

**a. Delta rules that do not start from the delta.** In step 14 (1 file), the cross part's join work is 4.24 M.
Two maintained strata take 62% of it:

| stratum | step 14 work | its whole cold work |
|---|---|---|
| `aknown__sup4__del` | 1,755,180 | 1,862 (`aknown__sup4`) |
| `rpvar__sup0__del` | 828,195 | 13,433 (`rpvar__sup0`) |

The plan seeds from `aleaf` (12,779 rows) and reaches the delta literal last:

```text
aknown__sup4__del(...) : aleaf($t,$m), fl412_acls($m,$a),
                         builtin_ref__old($h, 10121),   -- $h not yet bound: a cross product
                         pv__minus($a, 3689, $rr_w1),   -- the delta, empty or nearly
                         ahead($m,$h)
steps: 1 -> 12779 -> 2475 -> 1739925 -> 0
```

Across the step, 61% of the work is in rules whose delta literal is not first. In steps 07 and 08 that share is
16% and 8%: the blow-up depends on the plan each rule got, not on the edit.

**b. File-local rules rerun over the edited documents' whole rows.** These rules have no delta literal. They take
24% of step 14's work, 49% of step 07's and 70% of step 08's. Examples are `fl391_tgt`, `kind`, `ownq`,
`fl350_walk`, `docof` and `tv`. In step 14, `fl391_tgt` reads 33,477 `child` rows and `kind` reads 33,478 `node`
rows for the one edited file. Their cost follows the size of the edited files, not the size of the edit.

**c. Base rows churn because positions shift.** Over all edits, the counted rows changed as follows:

| relation | gone | new | paired by node | columns that differ in the pairs |
|---|---|---|---|---|
| `base:node` | 760,327 | 770,607 | 754,443 | `end_` 754,394, `start` 754,171, `kind` 14,494 |
| `base:node_order` | 477,604 | 487,884 | 471,720 | `k` 471,720 |
| `fl391_tgt` | 22,725 | 32,999 | 28,587 | c2 28,539, c1 28,290 |
| `own:edges` | 30,224 | 28,079 | 25,738 | c2 24,083, c1 20,232 |
| `base:child` | 20,623 | 30,897 | 5,370 | `c` 5,296, `idx` 3,998 |

A node's id `n` stays stable, but 98% of `node`'s changed rows differ only in their byte positions. Every insertion
renumbers `node_order.k` after it. So nearly every row of an edited file counts as changed. That feeds cause b,
and the deltas that every maintained stratum reads.

**The `RERAIL_SPIKE_DELTA` diagnostic is stale.** It reports "no delta" for every stratum and 0 rows over-deleted.
It counts strata of the original program, whose rules never read delta relations. It also looks for `__del`
relations that in-place maintenance no longer keeps. Repairing it would mean reading the maintenance program's
strata and the native maintenance counts.

## 2. The join loop

A sample of steps 06–08 (cold run, three sessions and the final full run): 7,735 work samples after dropping
waits.

| group | share |
|---|---|
| store (frames, `cross_rows::encode`, `keep_frames`, `realpath`, zstd, `Exports::add`, `Totals::add_rows`) | 19.7% |
| other (file reads/writes, `input_delta` FxHash sets, jemalloc decay, `str::parse`, `kept_source`) | 19.0% |
| joins: base views (`Batch::locate`, `Viewed::value_at`, `lookup_docs`, `Relation::between`, `Projected`) | 13.9% |
| joins: driver (`run_join_in_each::go`, candidates) | 11.8% |
| joins: probes (`Index::find_at`) | 10.5% |
| sqlite (86% file I/O; on the store writer's thread) | 8.1% |
| keeper statistics | 6.1% |
| sorts | 5.5% |
| parse | 2.7% |
| joins: dedup (`Fresh::settle`) | 2.6% |
| planning | 0.1% |

The joins come to about 39% of the samples. Within them, the base views are the largest share. Their leaves are
`View::find`, which reaches `Relation::between`, `Viewed::lookup_docs`, `Batch::locate`/`Batch::row` and
`Projected::local_of`, and `Rows::row`, which reaches `Viewed::value_at`. The syntax views resolve each row
through a batch and a document lookup on every probe, where a stored relation reads a flat column.

Pool spinning (rayon `steal`/`find_work`) is 36 samples (0.5%).

## 3. The cold phases

The phase counts are process-wide instructions, so they include every thread. The main thread does little in
these phases. In the profiled cold run, its CPU was 0.13 s for combine, 0.06 s for prepare and 0.00 s for wait.
Unprofiled, by process CPU (`RERAIL_TIMELINE`):

| phase | process CPU | made of |
|---|---|---|
| combine | 0.49 s (3.7–5.4 G) | the typing programs' run (its local part 0.33 s), then phase 1 (`sources.rr`: directory listing, digests, `realpath`, its own run, about 0.28 s wall), then reading the program (0.05 s) |
| prepare | 0.15 s (1.1–1.9 G) | main thread 0.06 s (prune and demand 0.018, push file-local joins 0.007, cut exports, guests); the parse and batch workers already running |
| wait | 0.18 s (1.7 G) | the parse workers building documents; the main thread only waits |

Combine has a wall gap of about 0.7 s with almost no CPU, between the typing programs' run and phase 1. It is not
yet attributed: no span opens on the test thread there.

Across the whole profiled run (cold, session and full), the store writer's thread took 0.65 s CPU, mostly SQLite
WAL commits from `write_back::start_writer`. On the engine thread, waits for the writer (`ContentStore::written`,
`unregister`) came to about 0.2 s. The longer waits (0.6 s) are for the cross part's stream workers, which is
expected.

## What this sets up

1. **Plan delta rules from the delta (cause a).** Every `__del`/`__ins`/`__plus`/`__minus` rule should start from
   its delta literal, or be planned with the delta's measured size. This is the largest single lever on small edits:
   62% of step 14.
2. **Positions that do not renumber (cause c).** Make `start`/`end_` and `node_order.k` relative, or move them into
   a relation few rules read. Then an edit changes only the rows it touches, and file-local reruns (cause b) can
   become deltas.
3. **Base views on probes.** Read the views' columns flat, without a batch and document lookup per probe (13.9% of
   samples).
4. **Cold combine.** Find the 0.7 s wall gap. Phase 1's file listing, digests and `realpath` are file-system bound.
5. Repair the `RERAIL_SPIKE_DELTA` diagnostic for in-place maintenance.

## Follow-up (2026-11-14, after decision 0217)

1. **Delta rules (cause a)** were not a search problem. The search priced `aleaf` probes as all its rows, because a
   relation held as kept had its row count but no column statistics. Decision 0217 gives it the engine's kept
   statistics. Step 14's cross part fell from 7.07 to 3.73 G; the 20 steps fell 3.8%. Rules whose delta literal is not
   first now take 1% of step 14's join work, from 61%.
2. **Cause b is not about positions.** The file-local strata the cross part reruns (`kind :- node`, `fl391_tgt :-
   child, kind`) read no position column. They are derived over each edited document's rows (33 k for one file),
   four loops a step. Their output deltas are tiny (`kind`: 173 gone, 25 new). After 0217 the whole step-14 join
   work is 1.03 M rows, so these reruns are no longer the floor. Making positions relative would change no one of
   them, and would change the rule packs' and findings' schema.
3. **The floor is now per-stratum overhead.** Step 14's session ran 308 strata. 128 of the 241 maintained relations
   changed nothing. Profiled CPU in the stream jobs: joins 33 ms, a stratum's setup (`spec`) 16 ms, planning
   searches 12 ms, live statistics 10 ms, index opening 9 ms, close 7 ms, reprobe 6.5 ms.
4. **Base views.** Flattening the views into columns would decode the tape, which the in-place rule forbids. In
   place, a stored relation's index search now reads a fence first (commit 9a47c0382); it did not resolve at load 83.
   `Projected::local_of` is a binary search over a document's id pairs, already contiguous.
5. **Combine's 0.7 s gap** was not idle. With every thread's spans (`RERAIL_TIMELINE=all`), the stretch from the test
   store's template run to phase 1 is 0.21 s, covered by reading and binding the 680-rule program. The 0.7 s was
   that work stretched by load.
6. **`RERAIL_SPIKE_DELTA`** now reads each reached stratum's form as it ran. Step 14: 241 relations maintained, 128
   unchanged; 2,498 rows over-deleted, 2,494 truly gone (4 put back), 497 inserted.
