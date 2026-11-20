# 0464: A batch makes a document when first read

Status: decided by the user (2026-11-19, Q285: "B: lazy Batch everywhere"). Builds 0458's "a document is made when
first read" for every batch, the chunks' too. Follows 0353 (a view is a read of the tape) and 0458.

## Intro

A batch (`syntax::Batch`) is a set of documents that rules read through: views such as `node`, `child` and
`field`, read off each document's projection, and relations a batch makes from the tapes, such as `text`, `op`,
the AST matchers' and the text tests'. Today `Batch::build` makes everything for every document before anything
is read: each document's projection (its rank index), every relation's rows, the span prefilter's marks, the
AST matchers and the text tests.

The cross part's batch holds every file of the run (0458). After a one-file edit the cross part reads it by
probes of the changed rows' nodes, which name a few documents, yet the batch makes all of them. On the 50
data-layer files' edit that is 0.37 s of the main run, and its versions then scan the relations that have no
index of their documents (0.15 s more).

## Decision

A batch holds its documents unmade, and makes each when it is first read.

- **A group is what is made together.** A read makes the documents it needs and are not made yet, as one group:
  their projections, then their rows. One span-prefilter launch, one AST wave and one interning call cover the
  group. A probe by a node or a document names one document; a scan, a `len` the batch cannot answer, or a device
  read makes every unmade document.
- **A view's rows are in document order.** Each document's rows lie at an offset from the counts of the
  documents before it: its nodes, children, fielded nodes, siblings and places. A count is the made projection's,
  or one the batch was given (below). An offset the batch cannot know makes the documents it needs.
- **A made relation's rows are numbered by group.** A group's rows follow those of the groups made before it.
  A batch read whole from the first read (a chunk's) makes one group, in key order: the same rows, the same one
  `Relation` per relation, the same block the device reads as today. A relation over several groups is ordered
  and in its block only when it has one group.
- **A probe is routed by what the declarations type.** A relation's row names its document by its first column
  where the rule library's declarations (`docs/rule-syntax/lib`) give that column one meaning: `n` is always a
  `node` or a `loc` (an id, `d << 32 | …`), `d` and `g` always a document's key. Other names mean different things
  in different relations (`p` is `token`'s node and `ast_match`'s pattern), so such a relation (`token`,
  `rx_span`, `ast_match`, `value_type`, …) is not routed: any read of it makes every document. No relation is
  named. A group whose rows of a routed relation name a document outside it fails, named.
- **What a relation's names are** is fixed by the projection and the program, not the documents: the batch
  knows them before it makes any. A group that makes a relation it did not name fails, named.
- **The text tests are what they mean.** A text test (0037) is a test of a string: a probe tests the probed
  string (each tested once a run, as now), and a scan lists the batch's strings that pass. Nothing of a test
  changes with an edit, so it has no version (`Plus`, `Minus`).
- **Counts are kept.** The documents' table (0458) keeps each document's counts: its views' rows and each base
  relation's rows. A corpus batch given them knows its row space and `len` without making a document; a document
  it then makes whose counts differ from those given fails, named.
- **An edit's versions** read the changed documents' rows where their group lies; no relation is read through
  to find them, and no kept statistic is measured again.

## Requirements

Each has its test before the code that meets it.

- **R1.** A batch's views and relations hold the rows the batch made eagerly held, for every shape and dialect
  the facts and corpus tests read.
- **R2.** A probe by a node or a document makes that document alone: over a batch of several documents, one
  probed, the others stay unmade.
- **R3.** A batch read whole makes one group: one prefilter launch, one AST wave, one interning call, and each
  relation one `Relation`, in its block.
- **R4.** A text test's probe answers the probed string, a scan the batch's passing strings; after an edit it
  has no version, and the cross part's results equal a run from scratch.
- **R5.** The documents' table keeps each document's counts; a corpus batch given them makes no unchanged
  document for its row space; a made document whose counts differ fails, named.
- **R6.** After an edit, a base relation's changed rows are found by group: no read-through, no kept statistic
  measured.
- **R7.** The 50 data-layer files: 589 outputs equal across the cold run, the edit, the warm run after it and
  the edited tree's cold run; the documents the edit makes are counted; a cold run's and the edit's
  instructions measured against 0458's.

## Steps

1. Unmade documents and groups in `Batch`: projections and rows made by group, views' offsets from counts. R1,
   R2, R3. Done (2026-11-19): `syntax::made`; `a_batch_made_by_group_holds_the_eager_batchs_rows` (R1, R3),
   `a_probe_makes_the_document_it_names_alone` (R2); the facts and logic tests pass (557). The views' offsets
   still come from made projections: step 3 gives them counts, and `Batch::lazy` stays false until then.
2. The text tests as tests of a string. R4.
3. The counts kept with the documents' table, and given to the corpus batch. R5.
4. An edit's versions by group; the kept statistics read, not measured. R6.
5. Measured. R7.
