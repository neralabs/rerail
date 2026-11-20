# 0494: A batch makes a document when first read

(0474 until 2026-11-20, here and on phased-facts: the hub's 0474 is a level's formulas as phases.)

Status: decided by the user (2026-11-19, Q285: "B: lazy Batch everywhere"). Builds 0458's "a document is made when
first read" for every batch, the chunks' too. Follows 0353 (a view is a read of the tape) and 0458. Numbered 0464 until
2026-11-19 (another branch's 0464 is the program image's).

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
- **A made relation's rows are numbered by group, in one block relation.** Each relation of a batch is one block
  relation (`Relation::grown`, 0431 §6), and every group goes on in it, in the order made: a group's rows are a
  range of it, never a relation of their own. The CPU, the JIT and the device read the one relation, in its block
  (0353); no reader concatenates groups. A batch read whole from the first read (a chunk's) makes one group, in key
  order: the same rows, the same block the device reads as before. Before every document is made, a probe reads the
  ranges of the groups it routes to; after, the relation and its own indexes.
- **A batch grows only once complete** (0431 §6, §8, merged 2026-11-20 from `final-rerail-shape-rr-align`). A
  round's documents, or a tape value admitted inside the fixpoint, are added only to a batch every document of which
  is made: its relations' rows are then a prefix, which the round's groups, or the values' rows, follow in the same
  block relations. The round's batch shares the earlier documents as made (their projections are made once) and
  holds its new ones unmade. A tape value is a batch of its own, made at once; its hosts are the chunk's documents.
  The strings, their atoms and the families' producers are the batch's and every batch it grows into (`Shared`): a
  producer keeps string indexes, and what it knows of a file serves the guests a later group or round adds. They
  are held only while a group's rows are interned, never while the pool runs a job that may read the batch. A round
  after a value was admitted is refused: the values' rows follow the chunk's.
- **A probe is routed by what the declarations type.** A relation's row names its document by its first column
  where the rule library's declarations (`docs/rule-syntax/lib`) give that column one meaning: `n` is always a
  `node` or a `loc` (an id, `d << 32 | …`), `d` and `g` always a document's key. Other names mean different things
  in different relations (`p` is `token`'s node and `ast_match`'s pattern), so such a relation (`token`,
  `rx_span`, `ast_match`, `value_type`, …) is not routed: any read of it makes every document. No relation is
  named. A group whose rows of a routed relation name a document outside it fails, named.
- **What a relation's names are** is fixed by the projection and the program, not the documents: the batch
  knows them before it makes any. A group that makes a relation it did not name fails, named.
- **The text tests are what they mean.** A text test (0037) is a test of a string: a probe tests the probed
  string (each tested once a run, as now: a round's batch keeps the earlier batch's probes, and a scan tests only
  the strings no probe or earlier scan tested), and a scan lists the batch's strings that pass. Nothing of a test
  changes with an edit, so it has no version (`Plus`, `Minus`).
- **Counts are kept.** The documents' table (0458) keeps each document's views' counts: its nodes, fielded nodes,
  siblings and places, the projection's own numbers. A base relation's rows are counted by its kept statistics
  (0458), which an edit patches by the changed documents' old and new rows (decided by the user, 2026-11-19: a
  group makes its documents' rows together, and no row says which document it is of in every relation). A corpus
  batch given both knows its row space and `len` without making a document. A document it then makes whose
  views' counts differ from those given fails, named; a relation whose rows, once every document is made, are not
  the count given fails, named.
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
- **R5.** The documents' table keeps each document's views' counts; a corpus batch given them and the relations'
  kept counts makes no unchanged document for its row space or `len`; a made document whose counts differ fails,
  named, and so does a relation whose made rows are not its count.
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
2. The text tests as tests of a string. R4. Done (2026-11-19): `made::TextTest` (step 1) answers a probe from the
   probed strings and a scan from the batch's; the edit's versions skip a text test (a decision says so).
   `made_batch_tests::r4_a_text_test_has_no_version_after_an_edit`: a join of two files' `text` tested in the cross
   part, three edits, each equal to a cold run's (it fails with the skip taken out).
3. The counts kept with the documents' table, and given to the corpus batch. R5. Done (2026-11-19): beside the
   table, `#base-counts` keeps each corpus document's key, digest and views' counts (`retain::put_base_counts`),
   written when a document's were not given; a run gives the batch those whose digest is the document's now. After
   the edit's versions the batch is given each base relation's rows by its kept statistics (kept, or patched by the
   changed documents); a made relation's and a `py_*` family's `len` answer by them until made. Tests:
   `syntax::tests::a_batch_given_counts_knows_its_row_space_making_nothing` (no document made for any `len`; both
   mismatches fail, named) and `made_batch_tests::r5_the_documents_counts_are_kept_and_given_to_the_corpus_batch`.
4. An edit's versions by group; the kept statistics read, not measured. R6. Part 1 done (2026-11-19): a source
   names a set of documents' rows where it knows where they lie (`Source::rows_of_documents`): a made relation by
   group (a group of theirs alone, else its routed rows), a view by its documents' offsets, an axis by its nodes'
   runs in the batch's solve; the wrappers pass it on. An axis's lookup by `a`, once the rows a scan reads are made,
   gives rows of that scan (each contained node's pair to `a`), not rows made again past `len`: `Before` ranks a
   lookup's rows among the scan's, and read the old documents' axis at a row made in neither (0458's versions over
   `within`, reached by no test before). Test: `made_batch_tests::r6_an_edit_reads_no_base_relation_through`
   (`child`, `field`, `within` read across files; no read-through; the edit's outputs a cold run's). Part 2: the
   import fixpoint's run lists its files inside another run's waves and reads no old source there (0458's open
   item), so its `doc` and `tape_of` were counted again. Done (2026-11-20, below):
   `r6_an_edit_counts_no_kept_statistic_again` passes.

   Part 2, decided by the user (2026-11-20: "Table keeps the places"). The run is phase 1's `load` run: its files
   are listed inside phase 1's waves, and its compare runs after its chunks, still inside them. Finding an old
   source, tape or memo entry takes SQL (its row names its segment), and 0104 has none in a wave. The documents'
   table is read there already (a kept fact, in the snapshot). So each row of the table also keeps:
   - its content's sha256;
   - its source's place in its segment;
   - its tape's hash (`HashKey::tape_identity`) and place, in the run's grammar.

   The writer resolves them as it writes the table (its transaction may read SQL), once this run's sources and tapes
   are placed. Inside the waves the compare reads with no SQL:
   - it compares a file's digest now with the row's;
   - it maps the places, each checked by the hash kept: a segment number is `max + 1`, so a removed last segment's
     number is used again, and a place gone, moved, archived or reused is a miss;
   - it finds the old memo entry by the digest in the snapshot (each path's newest entry under the plan: the old one).

   A miss there is "none", as before (what reads a base relation runs again). Before the first wave a miss is read
   through SQL, as before.

   As built (`content_store::document_places`, `KeptWhat::Documents`): a row is 23 words (the 7 given, then the
   sha256, the source's place, the tape's hash, place and length); a table of 7 (an older store's) reads with no
   places. A content the run gave by its sha256 (not stored then) and stored by the time the writer writes is kept
   by its id, as the compare before the first wave resolves it: inside the waves a file is found by its id, and
   with a row still by digest every file of a warm run's `load` compared as changed
   (`the_run_after_a_cold_one_with_tape_values_writes_nothing`: 4 writes). `memo_admitted` says whether the plan's
   entries are in the snapshot: where not, inside the waves the old entries are "none", not a SQL read.

   Rejected:
   - the store's places of every object held in memory (a read of every place each process start, for this one
     reader);
   - the read as work behind the run, through 0298 §5's exception (a second exception to 0104).
5. Measured. R7. Done (2026-11-20), on this Mac (M5 Max), the CPU only (no Metal, `RERAIL_METAL_MAINTENANCE=off`), the
   57 data-layer rule files over the 50 files, against 0458's binary (4289403855) on the same inputs that day:
   - 589 outputs equal across the cold run, the edit, the warm run after it and the edited tree's cold run; 48
     findings, 0458's.
   - The run first stopped twice: the gate refused an aggregate inside the parse demand's component (gated per key
     now, the user's choice: decision 0065 §2a, Q388), and a key of no columns (`Key([])`, 0361 step 6).
   - A cold run first took 378 G instructions, 28.6 s CPU, to 0458's 213.6 G, 18.9 s: each of the 301 text tests
     ran every test over each chunk's strings and kept its own (`TextTest::whole`, step 2's). Now each string is
     tested once against every test (`made::Tested`): 208.3 G (232.1 G repeated), 20.2 s CPU; 826 MB at most to
     0458's 743 MB.
   - The edit: 45.0 G to 0458's 35.2 G, but 3.64 s CPU to 3.59 s. Instructions and CPU part here: retired
     instructions vary by ~10% between runs of one binary, the pool's waits among them, so the CPU is what
     compares.
   Still above 0458, open: a cold run's +1.3 s CPU (the local part's `document rows / digests` under each chunk's
   measure, 0.21 s; the text tests and the transducer's matchers under it, 0.36 s; joins, +0.84 s); its 83 MB more
   at most; the edit's 0.21 s of well-founded proofs over the whole program for the measures' keys (0364 R6, not in
   0458: the cross part's own proofs find no key, so they are not the same). The prepare span is split so these
   show (`the measures' keys`, `the memo's program and digests`, …).
