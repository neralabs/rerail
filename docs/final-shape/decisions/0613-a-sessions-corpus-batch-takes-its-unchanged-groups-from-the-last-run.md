# 0613: A session's corpus batch takes its unchanged groups from the last run

Status: accepted (2026-11-22). Amends 0494 (a batch makes a document when first read) for a session's corpus batch.

## Context

A session holds its corpus batch's documents (their inputs, made projections, views' counts) from run to run
(0458), but not their rows. Each run's batch is new, and every document a read reaches has its rows made again. At
the data layer's 334-file history, step 02's edit derives one stratum again (`rr_demand_agg_rr_agg51`, priced
right: maintaining every stratum costs 6.3 s). That derive probes `docof` by every `ca_param` row and makes 253
unchanged documents' rows, 0.64 s of the cross part's 1.36 s (t43 step 21). Before 0458 step 4 a memo entry kept
these rows (copy 2 of 0353, removed).

0494 puts each relation of a batch in one block relation, every group a range of it. A changed document's rows
lie inside it, so the held relation cannot be the next run's relation as it is.

## Decision

- **The next run takes the held batch's unchanged groups.** A session keeps its corpus batch after the run. The
  next run's corpus batch is renewed from it (`Batch::renewed`) when its projection, matchers and relations are the
  same. A held group every document of which is in the new batch, with the same key and digest, is taken. Its
  documents keep their slots (their projections are not made again), and its rows are not made again. Every other
  document is unmade, as in any lazy batch.
- **The taken rows are read where they lie.** Each relation of the new batch starts as the held relation's runs of
  taken groups' rows: one segment each (`rows::Part` gains a first row and an end, `Part::range`), no row copied.
  The rows the run's own groups make go on in a relation of their own after them (a tail past segments that do not
  chain, as 0384's forks open one). Ids stay dense: a taken run's rows are numbered after the runs before it.
- **At most `MAX_SEGMENTS - 1` runs are taken.** A changed document's group splits the held rows. Past the limit,
  the runs with the fewest rows are left, and their documents are made again. So no relation's segments are merged
  (0353 step 6's copy). The batch's shape stays bounded over a long session.
- **A device read merges.** A relation of several parts is merged once when a reader binds it whole (`in_block`,
  0353 step 6, listed). A session's batch is bound so only once every document is made (`MadeRows::in_block` is
  `done()`'s). An edit's reads are probes, which read each row where it lies.
- **What groups share carries over.** The new batch shares the held batch's strings, atoms and producers
  (`Shared`): a producer's knowledge of a file serves the guests a later run makes. It also shares the files' lines
  that guests are placed by. A changed file's lines replace its old ones when it is projected. A renewed batch's
  vocabulary (its text tests' scan) is the strings its live groups used: each group of a session's batch records
  them as it is made.
- **The held batch is not kept by the new one.** Its parts keep their blocks, the slots their projections. The
  session holds one batch at a time.

## Consequences

- A session's edit makes only the changed documents' rows, and those of documents no earlier run made.
- A session holds its corpus batch's rows between runs: its footprint is the batch's.
- A new process (the kept route) still makes every document a read reaches: nothing of a batch's rows is stored.

## Requirements

- R1: a renewed batch's every relation has the rows a fresh batch of the same documents has (as sets), by probe and
  whole.
- R2: a renewed batch makes the changed documents alone where the held batch had made the others (projections and
  groups counted).
- R3: a taken run is read in place: the new relation shares the held relation's part.
- R4: a renewed batch's vocabulary is a fresh batch's.
- R5: a session's edit takes the held groups (profile decision "a session's corpus batch renewed (decision
  0613)"), and its outputs are a fresh run's.
