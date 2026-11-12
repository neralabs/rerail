# 0120: An edit reaches strata by their deltas, and a kept relation brings its statistics

Status: draft (2026-11-12). It designs 0117's steps 3 and 4. §3 is built as a spike (below); §1 and §2 are not.

## Intro

**The question (the user).** Design 0117's steps 3 and 4 and spike them, after merging the hub and measuring
again:
- step 3: the driver rewrites each stratum when it is reached, and the base program's passes are kept;
- step 4: old rows are read back only for the strata that are reached.

**The answer in short.**
- **On the P3 bench, steps 3 and 4 can remove about 1.9 s of an edit's 20–28 s.** The rest is joins, mostly in
  two recursive components, and the phases before the analysis. 0116 measured larger targets (rewrite 0.5–1.7 s,
  whole-program passes 1.8 s) on the data-layer pack, whose program has 46,494 strata. `calls_p3`'s has about 640.
- **Two thirds of the strata an edit reaches today have nothing to do.** Of 3,463 strata, 1,882 hold empty
  (every rule joins an empty relation) and 421 are quiet (their kept rows, unchanged). The run finds that at their
  turn and costs little there (about 0.15 s), but it rewrote, bound and read back all of them before it began.
- **Step 3: the executor walks the base program's strata and asks for each one's rewrite at its turn**, from the
  relations whose deltas it has seen to be non-empty. Its strata, loads and rule numbers are the prepared
  program's, kept as it is (0073).
- **Step 4: a kept relation's rows are admitted when a component that reads them opens**, not before the run.
- **Built first, as the spike: a kept relation's statistics are stored by its rows' digest.** They were keyed by
  its address in memory, so every new process counted 208 maintained relations again and measured 241 kept
  ones again on each edit (0.87 s). That is step 4's rule for statistics: a relation not reached is not read,
  and one reached is not counted again.

**How to read this record.**
1. What was measured, and where an edit's CPU goes.
2. Step 3's driver, and step 4's admission, as they fit this executor.
3. The spike: statistics by digest, built and measured.
4. What else the measurements found.

## 1. Measured

**The bench.** `benchmark_python_calls_program_corpus` over the 54 files of `dl-sub40`, with `calls_p3`,
typeshed and `RERAIL_SQL_STRICT=1`, debug build, unprofiled unless said.

**The recording was made again** with `fixtures/python/calls/generate_corpus.py`, Jedi 0.19.2 on CPython 3.13.18:
4,432 calls over 54 of 54 files, none excluded. The earlier recording was of a larger corpus and is gone. The
bench gives 2,987 agree with it, and so does the engine from before the hub merge (34e0c74f), so the merge
changed no answer. 4,815 was the old recording's figure.

| Run | CPU (user + sys) | Wall | Peak RSS | Load |
|---|---|---|---|---|
| Cold, after the merge | 47.2 + 2.6 s | 72 s | 571 MB | 66 |
| Cold, before the merge (34e0c74f) | 75.1 + 2.5 s | 62 s | 925 MB | 38 |
| Warm | 5.6 + 1.0 s | 18 s | 390 MB | 19 |
| Warm, `RERAIL_PYTHON_REUSE=on` | 5.5–6.6 + 0.9–1.2 s | 32–37 s | 385–405 MB | |
| First edit after cold (profiled) | 40.6 + 2.1 s | 50 s | 661 MB | 46 |
| Second edit (profiled) | 27.0 + 1.6 s | 33 s | 668 MB | 24 |

Each edit appends one top-level assignment to one file. The whole-run cutoff never applies to this program
(O-cutoff-phases), so the warm run is the same with and without it.

**Where the second edit's CPU goes** (own CPU of every span, 20.9 s; the profile adds the rest):

| Work | CPU |
|---|---|
| Joins: the cross part and the engine's runs | 7.9 s |
| Planning: join order search and strata | 1.6 s (O-replan-churn) |
| Parsing | 1.5 s |
| Moving rows to the cross part | 1.4 s |
| Opening inputs and building indexes | 1.3 s |
| Counting and measuring kept relations again | 0.87 s (§3) |
| Copying derived relations out | 0.37 s |
| Reading the kept part's rows | 0.26 s |
| Binding the kept relations | 0.25 s |
| The rewrite | 0.12 s |

**The strata the edit runs.**
- The struct phase's cross part runs 444 of its 480 strata: 378 maintained, 66 derived again. The inputs changed
  are `bound_in`, `child`, `field`, `fl710_vopen`, `litq`, `live_asg`, `look` and `node`.
- The analysis part runs 70 of 111.
- In all, the rewritten programs have 3,463 strata. 1,160 open a component, 421 are quiet and 1,882 hold empty.
- The quiet and empty strata cost about 0.15 s at their turn. What they cost is before the run: they are
  rewritten, their old rows bound and read back, and their kept relations counted.

## 2. The driver, and admission when reached

### Step 3: the rewrite of a stratum at its turn

Today `maintain::rewrite` loops over every strongly connected component the cut marks dirty (one that reads a
changed relation, transitively) and writes the whole maintained program. The program is then lowered, split into
strata again and run. At its turn, `Run::native_stratum_` drops each rule that joins an empty relation and holds a
quiet stratum as its kept rows (0101 §1). So the run already knows, stratum by stratum, which deltas are empty.
The rewrite knew only which relations might change.

**The driver's shape.**
1. **The executor walks the base program's strata**, from the prepared program (0073), whose strata, loads and
   rule numbers are made once and stored with it. Nothing is split again per run.
2. **It holds the set of relations whose delta has rows.** The changed inputs' deltas seed it, by their bound
   `r__plus` and `r__minus` (0117 step 2). A stratum's relations join it when their net deltas (`h__plus`,
   `h__minus`) have rows. This is the same test (`held(..).rows()`) that `native_stratum_` makes today.
3. **A stratum that reads nothing in the set is its kept rows.** No rewrite, no binding of `r__old`, no plan.
4. **A stratum that reads a relation in the set is rewritten there**, by the existing per-component body of
   `maintain::rewrite`, moved into a `Maintainer::stratum(scc, nonempty)` that returns its rules. Its pieces run in
   their fixed order: overdelete, survivors, rederive, insert, then the net deltas. That order is a property of the
   rewrite, so it needs no whole-program `front::strata`.
5. **A stratum derived again**, one the rewrite cannot maintain, runs its base rules, as today.

**Why it is exact.** The rewrite of a component depends only on which of its reads changed (0117 §2). A read
whose delta is empty contributes no rule over its delta, so rewriting the component with the set the driver
holds gives the rules today's rewrite gives, less those that join an empty delta. Those are the rules the run drops
at its turn today.

### Step 4: rows and statistics read when a component opens

`Retained::load` admits from the frames the old rows of every relation a dirty stratum reads, in one batch, before
the run. Under the driver, a kept relation is bound as a handle. Its rows are admitted when a component that reads
it opens: a reached stratum, or a quiet one whose readers are reached. A relation no reached stratum reads stays in
the store.

**The batch check moves to the relation.** `kept_strata::admission_matches` checks the batch against the frames
it was read from. Per relation, each admission is checked against its frame's digest. If the frames changed under
the run, the run derives again, as it does today.

**Statistics follow the same rule** (§3): a relation reached is not counted again, and one not reached is not read
to be counted.

## 3. Built: a kept relation's statistics by its rows' digest

**The fault.** A run keeps two kinds of statistics of a kept relation:
- those kept by delta (`retain::Patched`, 0096 step 4), of a maintained relation;
- those measured once (`retain::Measured`, 0094 §4), of a relation read as it was kept.

Both were keyed by the engine relation's address (`Arc::as_ptr`). A run read from the frames starts with neither.
So every command-line edit counts each maintained relation from its postings (`count kind`, `count docof` and
206 more), and measures each kept relation when first planned with (241 measures). The comment at the count said
"counted here once, the first run it is maintained". That held only within one process.

**The fix** (`retain/kept_stats.rs`):
- **At write-back**, each kept relation's statistics are a part of its frame (part −1; its rows are part 0), under
  its rows' digest, holding each name a run binds it as (`r`, `r__old`). A relation admitted with them at the same
  rows is not written again. No new store: the frames hold them as they hold the rows.
- **At admission**, the part is admitted in the same batch as the rows (`Retained::load`). A relation whose part's
  digest is its rows' digest takes the statistics, under its address in this process.
- **Under strict SQL.** A first form kept them in the store's statistics blob (`stat_blob`). Its read inside the
  waves was refused by `RERAIL_SQL_STRICT`. As a frame part they come from the snapshot read before the first wave
  (`preload_frames`), as the rows do.
- **Why it is exact.** Statistics are a function of the rows, and the digest names the rows. An entry whose digest
  no frame has any more never matches.
- `RERAIL_KEPT_STATS=off` keeps none, as before. `RERAIL_KEPT_STATS_TRACE` prints, for each admission, how many
  relations took statistics and why the others did not.

**Measured** (the P3 bench, profiled, at load 45–70; 2,987 agree each run). A new build is a new frame space, so
the first run of a build is nearly cold. The second run writes the statistics, and the third is the first that
reads them:

| The statistics of an edit | Before (second edit, 2026-11-12 06:12) | After (third edit of the build) |
|---|---|---|
| Patch the maintained relations' statistics | 0.48 s | 0.001 s |
| Relations counted from their postings | 208, 0.48 s | 0 (152 spans entered, 0.00 s) |
| Relations measured when first planned with | 241, 0.39 s | 63, 0.15 s |
| Statistics taken from the frames | none | 28 kept by delta, 150 measured |

So about 0.7 s of the 0.87 s is gone. The measures left are of relations no run had measured before: 421 of the
574 admitted carry no statistics part, as no plan read their statistics in the run that wrote them. The CPU of the
whole edit is not compared: at loads of 45–70 it varied by more than the saving.

## 4. What else was found

- **O-kept-inputs-stale** (closed the same day: an evaluated document's rows are this run's). After the merge,
  both reach-index invalidation tests failed: after an edit that shortens
  c.py, a run without a session keeps c.py's old line-2 rows. They pass with `RERAIL_KEPT_INPUTS=off`, so the fault
  is in 0117 steps 1–2's kept inputs. Both tests are ignored with the item's name.
- **O-replan-churn** (mostly closed: distributions compared in proportion to the nodes; 3,350 → 935). Each edit
  replanned about 3,350 rules whose kept order "grew over 4x, by over 10000 rows", in
  a new process every time. Plans are kept across processes (0116 step 2), but these are searched again on every
  edit. It is 1.6 s of the second edit.
- **O-cutoff-phases** (closed: an unchanged P3 run is 1.6 s of CPU). The whole-run cutoff refused any program that reads an earlier phase. So an unchanged run of
  `calls_p3` costs 5.5 s, where a single-phase program costs 0.71 s.
- **The cold run is cheaper after the merge**: 49.8 s of CPU against 77.6 s, and 571 MB at peak against 925 MB.
  The loads differed (66 and 38), so the instruction counts of a quiet machine are the check still to make.

## Prior art

- **DBSP** (Budiu et al., VLDB 2023): "a zero delta propagates zero" is the driver's skip. The driver is DBSP's
  incremental circuit evaluated operator by operator, with the operators made when reached.
- **Differential dataflow's arrangements** (McSherry et al., CIDR 2013): an arrangement is shared by its readers
  and carries its own index. Here a kept relation carries its statistics with its rows' identity, so a new reader
  in a new process need not rebuild them.
- **The spike's ledger** (`docs/rule-engine-direction/prior-work/ast-index-spike-history-index.md`): "An in-memory
  vocabulary silently fell back after reopen" (DH L2185-2189). The address-keyed statistics were this trap: a new
  process silently took the slow route.
