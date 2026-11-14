# 0115: What a run keeps for the next: rows where a maintained reader needs them, digests elsewhere

Status: draft (2026-11-11), for review. Only the measurements and one fix (below) are built.

## Intro

**The question (the user).** "How do we write over 192 MB for loading a few KB of files?"

**The answer in short.**
- The 20 files are 476 KB of source. The run queues 304 MB of frames and 138 MB of memo entries.
- The frames are the cross-file part's state, kept for the next run (decisions 0077, 0101). The part keeps the
  rows of every relation it derives: 15,632 relations and 21.7 M rows, for 73,608 syntax nodes. The run's output
  is 62 findings.
- The next run, after a one-line edit, reads back 16.8 M of those rows. A maintained stratum needs the old rows of
  its relations (`h__old`). It then writes 10.2 M rows again, because a derived relation is kept as one frame for
  all files, and one changed row rewrites the whole frame.
- **And maintaining costs more than deriving.** The warm run's cross part takes 15.0 s to maintain what the cold
  run derived in 3.1 s.
- **So the question is not "which rows are never read".** Maintenance reads almost all of them. It is which
  relations are worth maintaining, rather than deriving again when they are dirty. A relation that is derived
  again needs only a digest across runs, for its early cutoff (0077 §4). And a relation that is kept should be
  kept by document, so an edit rewrites only what it changed.

**How to read this record.**
1. The measurements.
2. A fix found on the way.
3. Every mechanism that already decides which rows of a relation are needed. They are many, and they should be
   one analysis.
4. The decision for what a run keeps.
5. The order of work.

## What was measured

The 20-file corpus (476 KB, 20 `.py` files), under strict mode, with the debug-profile build, the queue's cap at
4 GB (0114), and the export demand on (0099). A cold run, then a one-line edit (an assignment appended to
`_internal/connection_helpers.py`), then a warm run against the cold run's store.

| | Cold | Warm, one-line edit |
|---|---|---|
| Wall, CPU (user + sys) | 65.1 s, 79.5 s | 45.9 s, 52.9 s |
| Instructions | 1,077 G | 567 G |
| The cross part (its phase's CPU) | 3.1 s | 15.0 s |
| Frames queued | 304.1 MB, 36,536 frames, 15,632 relations | 153.8 MB, 5,040 frames, 3,969 relations |
| Kept rows read back | none | 16.8 M rows of 9,733 relations |
| Changed rows written | 21.7 M | 10.2 M |
| Findings | 62 | 62, the same |

**Correction (2026-11-11, later).** These runs had the profile on, which costs much more than it should:
- **Times.** It adds about 26 s of wall time and 258 G instructions to an edit's run. Unprofiled, the edit's run
  takes 17.3 s of wall time, 20.5 s of CPU and 300 G instructions. The counts above (rows, frames, strata) do not
  depend on the profile.
- **"Warm" here means after an edit.** An unchanged run takes the whole-run cutoff: 0.71 s of CPU. The profile
  disabled that cutoff until b8819272f.
- **Where the edit's time goes.** Decision 0116 analyses it.

What the cold run's 304 MB of frames holds, by kind of relation:

| Kind | MB | Rows | Relations |
|---|---|---|---|
| `rr_in_annotation_0_*`: relations derived again inside annotations | 86.3 | 5.9 M | 760 |
| Ancestor chains, one per rule (`p227_rr_up61`, `p228_rr_up61`: 430 k rows each) | 49.6 | 3.3 M | 823 |
| `__sup` helpers | 31.0 | 1.8 M | 6,967 |
| File-local parts read back (`fl`, `fz`) | 29.9 | 1.7 M | 1,553 |
| Export demand (`xd`, `dm`, `xk`; 0099) | 28.9 | 1.9 M | 3,058 |
| `rr_` helpers (aggregates, contexts, `nearest`, `inside`) | 26.5 | 1.8 M | 851 |
| Other derived relations | 21.3 | 2.7 M | 1,473 |
| Inputs kept again (`within`, `node`, `tape`, `text`, `line`, `child`) | about 20 | | 6 |

The top 100 relations hold 195 MB. Every value is queued as 8 bytes, though most are node ids that fit in 4.

## A fix found on the way: the input delta by document

Before the fix, the warm run took 238 s and 5,050 G instructions, against 47 s cold. 192 s of it was four rules of
the form `r__minus :- r__old, !r`: the diff of an input's old and new rows, one probe of `!r` per old row, each a
scan. All four came out empty.

| Rule | Probes | CPU |
|---|---|---|
| `rr_inside_ks32__minus` | 143 k | 69.3 s |
| `rr_inside_ks28__minus` | 134 k | 59.3 s |
| `rr_inside_ks23__minus` | 117 k | 45.7 s |
| `node__minus` | 67 k | 17.8 s |

The diff by rules is the fallback. An input's delta is meant to come from its documents (`retain::document_delta`).
That compares each document's digest, old against new, and takes only the changed documents' rows.

Since 4b4388d9f (2026-11-10, retained facts admitted in batches), the old rows restored from the frames named their
columns `c0`, `c1`, …. `document_delta` took the delta only when both sources named their columns alike, so every
input fell back to the diff by rules. Now it compares widths, since both sources are the relation's rows in its
declared order. The restored rows are also named as the program declares them. That is 238 s to 45.9 s, with the
findings unchanged.

Not yet explained in the warm run:
- the cross part takes 15.0 s maintained, against 3.1 s derived cold;
- the phases add up to 19 s of the 46 s wall (cold: 18 s of 65 s);
- the cross part peaks at 6.4 GB RSS;
- a second edit on the same store ends at strict mode: phase 1's memo records a source by reading
  `source_objects` inside the waves.

## Everything that decides which rows a relation needs

Each of these answers part of one question: which rows of a relation does some reader need, now or in a later
run? They were built one at a time.

| Mechanism | Where | What it decides | Granularity |
|---|---|---|---|
| `prune` (T's) | `front.rs` | the rules of relations no output reads are dropped | relation, static |
| Liveness (note t26) | `liveness.rs` | which facts some output row's derivation uses (a measurement) | row |
| Magic sets, `.demand` (T's `magic_sets`) | `magic.rs` | a relation is derived only for the keys its readers bind; negations are gated | key, per wave by document |
| Demand for negated relations (0048) | `plan/demand.rs` | a relation read only negated is derived for the keys its readers reach | key |
| Store, inline, or demand (0099 item 1) | `plan/keeps.rs` | a derived relation is stored, or recomputed inside its readers | relation, within a run |
| Projection, `count only` (0098 §1) | `simplify` | the columns no reader reads are dropped | column |
| An existence count is a negation (0099 item 2) | `simplify` | `count {r} = 0` is read as `!r` | literal |
| `covering` | `lower.rs` | context groups that only test that rows exist are dropped | literal |
| Factorized store (0099) | `typing::file_local` | an export crosses the chunk boundary as parts, not as their join | relation |
| Export demand (0099, 2026-11-11) | `file_local/export_demand.rs` | an export crosses only with the rows its cross-file readers can join | key |
| A search is demand rules (0107) | rules in `.rr` | a search answers only what is asked | key |
| The cone is magic sets, kept by delta (0108) | `answers`, `kept_strata` | the file system is read only where it is asked | key, across runs |
| Kept strata and a session's kept part (0077, 0101) | `kept_strata.rs`, `retain.rs` | every derived relation's rows and digest are kept for the next run | relation, across runs |
| The per-file memo (0108) | `memo.rs` | each file's exports and the base rows the cross part reads are kept | file, across runs |

Unmerged and relevant, on `jeremybarnes/Data-layer-rule-expressivity`:
- **31aea1a9a, 3d47054af, 4e0774680.** Which kept relations' rows a run loads: everything the views among them read,
  followed through views. Native answers count as inputs that can change. This is the block this decision changes,
  so it is merged first.
- **87537241d.** In `split_negations`, a reader whose demanded reads have constant keys seeds `demand_<rel>` from
  them. Without it, a true Boolean came out false. A correctness fix.
- **4367e310b, 5b137a833.** A stratum with literal facts is not dropped once its rules are pruned.

**The shape they should share.** Two halves:
- **One demand analysis over the program.** For each relation it gives its readers, the columns each reader
  binds, and whether each read is positive, negated or counted.
- **One price.** 0099's: the work to build a relation, plus `M` per row to store, index and maintain it.

Each mechanism above is then a choice that analysis prices:
- no reader: prune;
- few bound keys: demand (magic sets, export demand, searches);
- a cheap body per probe: inline;
- columns unread: project;
- across runs: what is kept. That last choice is the one this decision adds.

The mechanisms are not merged into one pass here. But each new one is written as a consumer of that analysis and
price, not with its own.

## The decision

**1. A derived relation keeps, across runs, either its rows or only its digest.**
- **Rows** let it be maintained by delta (0077 §3): DRed reads `h__old`.
- **A digest** lets it be skipped when clean (0077 §4's early cutoff, which compares digests, not rows), and
  derived again when dirty. That needs only its inputs.

A relation kept by digest also passes on a cost. A dirty relation kept by digest has no old rows, so its delta
has no old side. Its maintained readers then derive again too, unless it comes out with its old digest.

**2. Rows are kept where a maintained reader needs them, and where maintaining is cheaper than deriving again.**
- Always rows: the outputs, and the relations whose own stratum is maintained and is read by a maintained
  stratum.
- Only a digest: a relation derived again in every run that changes it. Those are the strata
  `maintain::rewrite` lists as derived again: aggregates over changed groups, `nearest` after the tree changed,
  gated programs. Also only a digest: a relation whose every reader is derived again, and a relation no stratum
  reads.
- Otherwise, priced as 0099 prices storing: keep the rows when deriving it again costs more than maintaining it,
  plus `M` per row kept. Both costs are observed: the cold run's work and the warm run's (`native.tsv`). The `M`
  is 0099's. At 20 files the observation so far favours deriving again: the warm cross part is 15.0 s maintained,
  against 3.1 s derived.

**3. A derived relation whose rows are per document is kept by document.** Inputs are already kept by granule of
documents (`retain.rs`). Derived relations are one frame each (`DOCS_PART`), so an edit of one file rewrites
every file's rows of each relation it changes: 10.2 M rows for a one-line edit. A relation whose first column is
a node id holds rows by document. Its frame is split by the same granules, and a run writes only the granules
whose rows changed.

**4. The inputs are not kept twice.** `within`, `node`, `tape`, `text`, `line` and `child` are in the per-file
memo. The cross part's frames refer to the memo's entries, by document, instead of copying their rows.

## Order of work

1. **Built: the input delta by document** (above).
2. **Merge the Data-layer-rule-expressivity fixes** listed above, with the full suite after.
3. **Measure, per relation of the warm run:** rows read back as `__old` by a maintained stratum, rows of strata
   derived again, and rows no stratum read. Split the 154 MB of the warm run's writes the same way. This sets how
   much items 2 and 3 of the decision each save.
4. **Digests only, where the decision says** (decision item 2), with a profile line per relation and its reason,
   and `RERAIL_KEEP_ROWS=all` to keep everything for comparison.
5. **Derived frames by document granule** (decision item 3).
6. **The inputs from the memo** (decision item 4).
7. **Gates:**
   - the findings are the same cold and warm;
   - a one-line edit's writes scale with what the edit changed, not with the corpus;
   - the warm run is no slower than today;
   - measured at 20, 50 and 100 files.

## Open questions

- **The annotation copies.** Why are `rr_in_annotation_0_*` relations whole copies of their outer relations (86 MB)?
  If they are a phase's namespaced copy of the same rows, they could be a view of them.
- **The ancestor chains.** Should the per-rule ancestor chains be one shared relation (the planner's CSE), or a view
  of `within`? Either takes 50 MB out of what is kept, and out of the run.
- **The values.** Should the kept values be 4 bytes, where every value of a column fits? That is a store format
  change (0104), and is independent of this decision.
