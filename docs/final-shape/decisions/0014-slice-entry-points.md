# 0014: The slice's entry points keep the wave's rows per scope; a warm run evaluates changed documents only

Status: accepted
Date: 2026-10-30

## Question
06 S6: `slice::audit` and `slice::apply` take a scope from a root tree to its finding keys. Cold
and warm are meant to be one path. What does a warm run evaluate, and where are the match facts
of the documents it does not evaluate?

## Decision
- **One path.** Both calls:
  1. push the root through the families (S3);
  2. run the wave (S4);
  3. run the findings layer over the wave's relations (S5).

  `audit` is cold: it evaluates every document of the root, whatever was kept. `apply` takes the
  edited snapshot's root. S3 already has worktree edits reach the slice through a directory
  snapshot, and the store has no tree edits.
- **The wave's rows are kept in the store**, per scope:
  - `slice_match` and `slice_span`;
  - `slice_document`: each document's key and core digest at the last run;
  - `slice_rules`: the clauses' identity (`Layer::digest`), because a row's clause is an index.

  They are tables of the store's own file, on the wave's lease, so a warm run needs no process
  state. A scope with no kept run of the same rules runs cold.
- **A warm run evaluates the documents whose core digest changed** (or that are new). Its
  candidate paths are the kept documents and the observer's tree diff from the last root.
- **The wave's relations hold every document's rows.** `Wave::kept_program` adds two batch inputs
  of kept rows, so `match` and `span` hold the kept rows of the documents not evaluated besides
  the batch's. The families are recorded from every document's core digest (`schedule::run_over`).
  So an edit's family digests compare whole relations, and an unchanged family cuts off.
- **A run that evaluates no document** (unchanged, or only removals) is still one wave: the
  scheduler runs one empty chunk, whose batch steps read nothing, so the kept rows are the
  relations.
- **The layer runs over every document.** Its facts are each document's comments, lines and
  error regions, and a report ceiling counts over the scope. Its families cut off an unchanged
  finding set.

## Prior work consulted
- **Hist-index 2.20, "byte identity of incremental vs one-shot encode" (DH L51-54; L3353-3357):**
  a representation must not vary with batching (R17). *Agrees.* The check compares every warm
  run's keys and coverage with a cold audit of the same snapshot in a fresh store.
- **Design-index 10 and 20 (D S124 L512-592):** incremental maintenance allows no statistic that
  needs the whole corpus. *Partly.* The wave's per-family row estimates and the layer's report
  ceilings are recomputed over the kept and new rows together. That is a scan of the scope's
  rows, not a re-evaluation. S8 measures whether it matters.
- **Design-index 12 (D S145 L1087-1117):** partitioning pays its fixed cost once per part.
  *Agrees.* A warm run is one chunk of the changed documents, not one per file.

## Consequences
- **Checked by `slice::tests::an_edit_reruns_its_file_and_a_whitespace_edit_cuts_off`:**
  - a one-file edit re-runs that file's four derivations and evaluates only that file; the other
    files' findings do not change;
  - a whitespace-only edit evaluates the file, because its core digest changes, but its clause
    families cut off at the matches (only `span` changes), and so do its findings;
  - an unchanged snapshot and a removal evaluate nothing;
  - every warm run's findings equal a cold audit's.
- **The layer's facts are rewritten for every document on each run.** This is what bounds a
  warm run as the scope grows. S8 measures it; keeping those facts per document as well is the
  next step if they dominate.
- **No production caller yet.** S7's differential gate comes before the napi method.
