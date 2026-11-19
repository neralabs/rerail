# 0458: The cross part reads the base relations off every file's tape

Status: decided by the user (2026-11-19, Q285: "Read the tapes"), over the 2026-11-15 answer to Q2/Q5, whose
widening (0286) stays for the requests it covers. Follows 0353 (a view is a read of the tape, never a relation);
amends 0117 step 1 (the kept inputs) and 0401 F6 (what the keep writes) for base relations.

## Intro

A program's rules split into a file-local part, run per file in chunks, and a cross part, run once over every file
(0083). A cross rule may read a base relation: one the tape answers, such as `node`, `child`, `field`, `line`,
`col`, `docof`, `ast_depth`, or one a chunk makes from the tape, such as `text` or `kind`. Today the cross part does
not read those from the tapes. It reads three copies of their rows:

1. **The run's gathered columns.** Each chunk's rows of each base relation the cross part reads are gathered into
   columns (`base_rows`), and the cross part binds a source over them (`columns_source`).
2. **The memo entries' base rows.** A file read from the per-file memo has no chunk, so its entry keeps its rows of
   every such relation (`memo::base_key`), every column, and hands them on as an evaluated file would.
3. **The kept inputs' granules.** The keep writes each base input's rows into the space's chain, by granule, so the
   next run binds them where they lie (0117 step 1, 0401 F6). On the 50 data-layer files: about 5 MB of the chain's
   17.9 MB of values (`text` 0.94, `node` 0.87, `child` 0.76, `kind` 0.55 MB, then `line`, `field`, `col`, `tape`,
   `docof`, `ast_depth`, `visit_index`), and an edit rewrites the changed file's granules.

None of the three is in 0353's list of copies. Each is a view's rows made into a relation, which 0353 forbids.

## Decision

The cross part binds each base relation it reads as the chunks bind theirs: over a batch of documents
(`syntax::shaped`), its views read in place off the tapes. The batch is every file of the run, each document made
from its stored tape, as a chunk makes one. Nothing keeps a base relation's rows.

- **The corpus batch.** Every file's documents (its own and its guests'), keyed by their path atoms as in the
  chunks, so a node's id is the same in both (0083 §3, 0094 §2). A file the chunks evaluated hands its document on;
  a file read from the memo has its document made from its stored tape and source (a load, not a copy, 0353 rule 3).
  Everything it needs from the store is found before the first wave (0104).
- **A document is made when first read.** A batch makes each document's projection as it is built
  (`Batch::build`); the corpus batch makes a document's when the cross part first reads that document. An edit's
  maintain mode reads by probes of the changed rows' nodes, so it makes the documents those name, not all of them.
  A scan of a base relation reads every document, as it read every row before.
- **Guests' tapes are stored again,** named by the run's manifest. Q30 stopped writing them because no later run
  read them ("named by the manifest, if a later run reads them"); the corpus batch is that reader. A file read from
  the memo has its guests' documents made from their stored tapes, not parsed again.
- **An edit.** A base relation's change is the changed files' documents: its old rows are their old tapes' rows,
  its new rows their new tapes'. The maintain mode takes them as the given versions of the read (0364 R1, R2):
  `Minus` over the old documents, `Plus` over the new, `Old` the batch before the edit. No rows are diffed.
- **What is kept of a base relation.** Its documents' digests, so the next run knows which changed, and its
  statistics (0120 §3). No rows, in the chain or in a memo entry.
- **A session** (0077) holds its corpus batch from run to run, its changed files' documents replaced.

The copies that a chunk itself still makes from the tape (0353's copy 7: `text`, `kind`, `visit_index` and the
other eager rows) are made by the corpus batch the same way, and go with 0353 step 3. They are not kept.

## Requirements

Each has its test before the code that meets it.

- **R1.** On a cold run, each base relation the cross part reads has the rows it had over the gathered columns, for
  every base relation and dialect the data-layer and corpus tests read.
- **R2.** A file read from the memo gives the cross part the rows an evaluated file gives, read off its tape.
- **R3.** After an edit, a base relation's versions are the changed files' old and new rows (`Minus`, `Plus`), and
  the cross part's results equal a run from scratch on the edited tree.
- **R4.** No base relation's rows are written to the chain or to a memo entry: a guard over the keep and the memo
  writer, and the chain report (`chain_bytes_by_relation`) lists no base relation.
- **R5.** The strict run (`RERAIL_SQL_STRICT`) passes: no SQL inside the waves.
- **R6.** The 50 data-layer files: all 589 outputs equal across the cold run, the edit, the warm run after it and
  the edited tree's cold run; the chain's bytes, a cold run's and the edit's instructions measured against today's.

## Steps

1. The corpus batch: every file's documents, a memo hit's (and its guests') made from their stored tapes, found
   before the first wave; each document's projection made when first read. R2.
2. The cross part binds its base relations over the corpus batch, in place of the gathered columns; copy 1 goes.
   R1, R5.
3. An edit's versions of a base relation from its old and new documents. R3.
4. The memo entries' base rows and the base inputs' granules go; copies 2 and 3 go. R4.
5. Measured. R6.

## Open

- A run of many files holds every file's tape mapped for the cross part. Their bytes are the tape store's, mapped,
  not the heap's; the peak is measured in step 5.
