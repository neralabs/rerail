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
   Then above 0458: a cold run's +1.3 s CPU, its 83 MB more at most, the edit's 0.21 s of well-founded proofs. The
   prepare span is split so these show (`the measures' keys`, `the memo's program and digests`, …).

   Measured again (2026-11-20, evening). Those runs had not been the CPU's alone: with `RERAIL_METAL_MAINTENANCE=off`
   both engines still submitted Metal work on this Mac, and 0458's has no switch that keeps it off. Both now run
   under a sandbox that refuses the GPU's user clients (`AGXDeviceUserClient`, `IOGPUDeviceUserClient`: no process
   can make a Metal device), ours with `RERAIL_GPU=cpu` as well. Found and fixed, each with the 589 outputs unchanged:
   - **The cycles, not the instructions.** About 2× 0458's cycles for 6% more instructions: the tape values' parts
     (`syntax::values::Values`, 0431 §8) were behind a read-write lock that every probe of a layered source took,
     21% of the samples in `lock_shared_slow`. They are an `ArcSwap` now: a probe loads them, and an admission makes
     a new map under its own lock. That map is made from the last without copying it (2026-11-21): the parts are
     blocks of 64 and the keys 256 shards, each shared by the maps that hold it, so an admission copies its
     relations' last blocks and its keys' shards. Copied whole (`Arc::make_mut` on parts the probes' map shares),
     each admission copied every part and key before it: 7.6 s of a 90 s run on corpus0 × B-now, the normal packs
     2.605 T to 2.189 T instructions (test: `an_add_copies_only_its_last_block_and_its_keys_shard`).
   - **The proofs.** The whole program's well-founded check found the recursive components by a pass over the
     6,842 rules for each component: 0.165 s, in both the cold run and the edit. One pass over the rules, keyed by
     each relation's component: 0.056 s (the helpers' steps 0.025 s of it, checking every recursive component under
     0.03 s).
   - **The text tests' lazy DFA.** A group the DFA limit leaves lazy was searched in pieces on every thread, each
     thread building its own states (3.6 µs a string over an edit's 30,000), and the regex's own pool kept a cache a
     thread for the vocabulary's life (36 MB at the peak). A lazy group is searched on one thread, with a cache of
     that search's own; a determinized group is still read by every thread at once.
   - **A sketch made for every column measured** (`syntax::measured`), used only when a shadow is written: made only
     then.

   Cold, three alternated rounds (load 6–7): 12.35–12.44 s CPU and 202.4 G instructions to 0458's 12.02–12.08 s and
   195.3 G (+2.8%); 726–732 MB of footprint at most to 594–610 MB. The edit: 3.42 s CPU to 3.04 s, 51.4 G to 41.4 G
   (prepare +3.0 G, the chunks +2.2 G, the cross part +2.1 G).

   Open: the footprint's +125 MB. It is not the CPU lane's 64 MB tape slab (a run with no device at all peaks
   higher), nor the heap at the peak (within 20 MB of 0458's, the slab's zeroed pages aside), nor the blocks' free
   lists (their free bytes are pages given back; with no split of a fitted chunk the run is the same).
   That last was wrong: the free lists held 316 MB at the end of a cold run, about 200 MB of it resident (step 6).
6. The footprint: the blocks' free lists. R7. Done (2026-11-20, user: "please do spike 3"). A free chunk's first
   word held its list's link, so its first page stayed; free chunks never merged, so one under a page never freed
   one. Three spikes, each with the 589 outputs unchanged, three alternated rounds (load 9–29, footprint at most):
   | | HEAD | links outside the chunks | small structures on the heap | buddies, links outside |
   |---|---|---|---|---|
   | footprint | 703–728 MB | 646–666 MB | 693–718 MB | 631–650 MB |
   | peak RSS | 891–900 MB | 889–895 MB | 858–860 MB | 830–836 MB |
   | instructions | 202.7–203.8 G | 202.8–203.9 G | 202.4–202.7 G | 202.7–203.5 G |

   The small structures were the planner's and setfix's sketches, statkeep's `Exact` and `Groups` (a census of the
   chunks under a page: the relations' rows about 97 MB of them, the sketches about 88 MB, `Exact` 30 MB, `Groups`
   14 MB); on the heap they saved 10 MB, and splitting no fitted chunk under a page saved none. Built: the third,
   decision 0342 as amended (the lists in the ledger, a page given back once no live chunk shares it, buddies
   merged, a chunk from the front aligned to its size up to 1 MB). Near the peak the free-listed blocks hold 80 MB
   live in 84 MB of pages holding a live byte; the rest is given back.

   Cold, three alternated rounds (load 17–25): 648–656 MB of footprint at most to HEAD's 714–724 MB and 0458's
   596–608 MB; 202.9–203.1 G instructions to HEAD's 202.4–202.7 G and 0458's 194.5–195.3 G.

   The 40–50 MB left are the blocks', not the heap's. `vmmap` taken back to back through a run of each, the five
   samples of most footprint: the heap (jemalloc's region) 362–378 MB dirty to 0458's 360–381 MB, the blocks 135–169
   MB to 0458's 112–134 MB. The heap's 16–23 MB more in single samples before was their timing. A profiled run's heap
   by span does not say it either: its snapshot of the strata under way moves by tens of MB between runs, and the
   profile's own author-join sources (`rr::profile_join_sources`, 7 MB) are counted in ours alone. Open: the blocks'
   35 MB at the peak.

   That gap is explained (a census of every block at the peak, two runs of ours against three of 0458's, the 50
   files on the CPU): the free-listed blocks hold 114–140 MB live to 0458's 80–86 MB, and waste nothing (0458's
   lists held 30–50 MB of pages besides); the plain blocks with no free list hold 23 MB to 0458's 1 MB (the
   containment solve 10 MB, the batch's block 5 MB, atoms 4 MB). The live difference is data that left 0458's heap
   for blocks (sketches, statistics, atoms, the solves), the column filters (11 MB, new, 0477), the relations' own
   index buckets (5–8 MB), and the solves grown on the way (16 B a row where 0458's heap held 8 B a pair).
7. The axes' solves: 8 bytes a row, shared within a batch. R7. Done (2026-11-21, user: "build A, and fold in sharing
   the repeated solves"). An axis is file-local: a solve row's `n` and `a` lie in one document, and an id is its
   document's key over its node's low 32 bits. A 4-byte column may now hold a value's low bits under key runs
   (`rows::ColDir::runs`: a table in the relation's block of each run's first row and high bits, one run a
   document); every read (`ColDir::get`, CLIF's `col_get` and its search `col_high`, the kernels' relation.h)
   returns the value, so no reader changed. A table of each node's id was rejected: it is the node view's id
   column made, a copy of what the projection's numbering holds (and the chunk's `low`, a listed copy, 0353 step
   4). The host's and the device's writers write the solves so (`Part::over_key_runs`): 3.06 M rows over the 50
   files, 24 MB where they were 49 MB, each read a 4-byte load and a search of a few runs in cache.

   Of those rows 155 k (5%) were solves made twice: 27 k an `inside` whose set nests nowhere, the same rows as its
   batch's `nearest`; 128 k two `nearest` solves of the same documents in two phases' runs (phase 1, phase struct
   and the main run each project their documents). A batch's axes over one set of kinds share their solves now
   (`within::Solves`): each mode's is made once, and an `inside` that nests nowhere reads its `nearest`'s relation.
   Open: the phases' solves of the same documents, which need a solve held per document across the phases' runs
   (rows in its tape's ids, keyed by its digest), not per batch.

   The column filters stay at 8 bits a key (user, 2026-11-21): at 4 bits they took 5 MB to 11 MB at the peak, with
   up to six times the false positives (20% at the outgrow point to 3.3%), and no run of these files moved.

   Cold, two alternated rounds (load 6): outputs identical; 216.6 G instructions to 216.8 G before the sharing,
   764 MB of footprint to 762–777 MB.

## Reads side by side make a document once (2026-11-20)

A group is made unlocked (its prefilter and matchers run on the pool), and it was kept only if no other read had
made one of its documents meanwhile. With the base relations counted on the pool (Q396, 0487's merge), every count
read the whole batch at once: each made every document, and all but one set their groups aside. On the data layer's
`leg-083-typed` over 901 files that was 3,604 documents' rows for 901, 835 G instructions and 2.6 GB at the peak.

- **Claims.** A group's documents are claimed by its thread while it is made (`State::claimed`). A read finding
  documents another thread claimed waits for them: it runs the pool's next job (`rayon::yield_now`), else waits for a
  group to end (1 ms at most a wait), and reads them once made. A claim is given up when its group ends, made or not
  (`Claim`'s drop, a panic's too).
- **No maker waits.** A thread making a group of the batch (`State::makers`) makes what it reads itself, as before:
  a read under it, a job it ran while its group's jobs ran, never waits. So no two makers wait on each other.
- **Checks.** `syntax::tests::reads_side_by_side_make_each_document_once`: a thread a relation, each reads it whole at
  once, and every group made is kept (`Batch::makes` is `Batch::groups`).
- **The counts in waves.** The counts side by side each list their relation's rows (`RelStats::count`, 48 bytes a
  row), so counting every relation at once held 1.2 GB more. They are counted in waves within a share of the
  budget (`budget::share(256 MB)`, decision 0126), each wave side by side.

The same run, CPU, unprofiled: 581 G instructions and 1.36 GB at the peak, 18.2 s (the hub before Q396's pool counts,
measured the same way: 534–540 G, 1.14–1.15 GB, 28–30 s at a higher load). Cycles: 164 G to its 194–207 G.
