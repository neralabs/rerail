# 0117: A relation carries its delta; maintenance is the core's, driven by the deltas that are not empty

Status: accepted (2026-11-11). Step 1 (the inputs from the changed documents) is being built; steps 2–4 are not.

## Intro

**The question (the user).** Should maintenance move into the core, and be managed with subsidiary, optimized,
less generic data structures?

**The answer in short.**
- **Yes. Maintenance becomes the core's driver for maintained strata, not a second evaluator.** It uses the same join
  specs, planner, plans (now kept across processes, 0116 step 2) and native strata as every run.
- **The subsidiary structure is a relation's delta, held beside the relation.** It has three parts:
  - the rows dead in place;
  - the rows appended;
  - each document's digest of its rows.

  "Is this delta empty?" is then a field read, not a query, and so is "which documents changed?".
- **What it removes is work on what did not change.** An edit's run today:
  - rebuilds every input from every file's memo entry (19 unchanged entries, 135 MB);
  - hashes every input by document;
  - rewrites the whole program for maintenance (203,141 rules, 46,494 strata, 39,640 of them over empty deltas);
  - reads back the old rows of every relation of a stratum that runs.

  With the delta on the relation, each of these touches only what changed.

**How to read this record.**
1. The structure.
2. The driver.
3. Why each is exact.
4. Prior art and the spike's lessons.
5. The order of work, with the measured targets from 0116.

## 1. The structure: a relation with its delta beside it

A maintained relation is its **kept rows**, read in place from the frames (0099, 0104), plus side structures:

| Side structure | What it holds | What it answers |
|---|---|---|
| Dead rows | Bit per kept row, set by the run (0090 step 4's survivors) | The relation's rows now, without a copy |
| Appended rows | Columns of the rows the run added | The insert delta, `r__plus` |
| Removed rows | The dead rows, as a view over the kept columns | The delete delta, `r__minus` |
| Document digests | Per document: the sum of its rows' hashes, and their count (`retain::DocDigests`) | Which documents' rows changed; the relation's digest (`digest_of_docs`) |

These are the structures a run already has, but in separate places:
- the session's counted exports (0096 step 3);
- the kept part's input documents (`Retained::input_docs`);
- the survivors' dead rows (`Start::Base`);
- the `__plus` and `__minus` relations, materialized as relations of their own.

Here they are one value with one owner, so every step reads the delta where it is and never rebuilds it.

**A delta is O(changed rows) to make and to test.** Its size, and whether it is empty, are counts on the
structure. The relation's digest after an edit is the kept digest, less the changed documents' digests, plus their
new ones. The digest is a sum over rows, so this is exact.

## 2. The driver: strata in order, with the set of non-empty deltas

0101 §1 already skips a maintained stratum whose every delta is empty, as the run reaches it. 0116 measured what the
skip leaves behind: the whole maintenance program is still built, keyed, split into strata and passed over, about
3 s of a 20 s edit. The driver moves that work behind the skip:

1. **Walk the base program's strata in order**, with the set of relations whose delta is not empty. The inputs'
   deltas seed the set (step 1, below).
2. **A stratum that reads nothing in the set is not touched.** It is not rewritten, planned, opened or read back.
   Its relations are their kept rows, with empty deltas.
3. **A stratum that reads a relation in the set is rewritten there and then**, for the positions whose delta is not
   empty (`maintain::rewrite` of that stratum alone, under counting by default, delete and rederive for recursion).
   It is planned with the kept plans' delta-first orders (`first`, 0116 step 2), and run. Its relations' deltas are
   then the side structures above, and are added to the set when they are not empty.
4. **The whole-program passes are the base program's**, made once and kept with the prepared program (0101 §2's
   "better" form). Strata, first and last reads, loads and rule numbers are computed once, not each run.

**Why it is exact.** Every rule the rewrite writes for a stratum has a delta at the position that changed (0077
§3). Over empty deltas it derives nothing, and its survivors are the kept rows. A stratum's rewrite depends only on
which of its reads changed, so rewriting it when it is reached gives the same rules as rewriting the whole program
first, and then keeping that stratum's.

## 3. Step 1: an input is its kept rows plus the changed documents' rows

An input of the cross part is an export or a base relation: rows the file-local part makes for each file, kept in
that file's memo entry. Today a command-line run rebuilds each input from every file's entry. The session's shortcut
(the counted exports, 0096 step 3) lives only in the process's memory, so a new process never has it.

The kept part's frames already hold each input as the last run had it, with its document digests. So:

- **An unchanged file's memo hit is not replayed.** Its export and base rows are already in the kept input. Its
  lines (the file-local outputs) are still taken from its entry, decoded alone.
- **Each input is its kept rows with the changed documents' rows dead, plus the rows of the files this run
  evaluated.** The changed documents are those the statistics keeper counted in or out.
- **The input delta is those rows: gone and new.** It is handed to the maintenance rewrite as given (0090 §2), with
  no hashing of unchanged documents.

**When it is exact.** A row belongs to the document its first column names (`memo::document_of`), the rule that
`retain::document_delta` and `input_delta::delta` already use. It is exact for an input none of whose changed
files' rows name another document. If any row does (a value document's row, which several files hold), or if the
kept part is missing or was not written by the run the keeper counted, the run takes today's route for that run:
every hit is held from its entry, as the session's "hold after all" does. So it never answers wrong, and falls back
to the measured cost.

**Checked by `RERAIL_CROSS_VERIFY`.** It builds both forms and compares them row by row.

## 4. Prior art, and the spike's lessons

- **DRed and counting** (Gupta, Mumick and Subrahmanian, *Maintaining Views Incrementally*, SIGMOD 1993): the
  rewrite rules this engine already uses (0077, 0090). Unchanged.
- **Differential dataflow's arrangements** (McSherry, Murray, Isaacs and Isard, CIDR 2013): an indexed collection
  shared by its readers, plus batches of changes, each batch merged in when it is small. The relation here is the
  same idea, at the run's grain: kept, indexed rows plus one batch per run. Unlike differential dataflow, there is
  no timestamp lattice. A run is one version.
- **DBSP's Z-sets** (Budiu, Chajed, McSherry, Ryzhyk and Tannen, VLDB 2023): a delta is a weighted set, and the
  incremental form of a query applies the query to deltas. The document digests are a Z-set's sum, by document. The
  driver's skip is DBSP's "a zero delta propagates zero".
- **Soufflé's incremental evaluation** (Zhao, Subotić, Raghothaman and Scholz, *Towards Elastic Incrementalization
  for Datalog*, PPDP 2021): counting per tuple, with the incremental program generated beside the base one. The
  driver here generates it per stratum, when reached, which is the departure: their whole-program generation is the
  cost 0116 measured.
- **The spike's ledger** (`docs/rule-engine-direction/prior-work/ast-index-spike-history-index.md`):
  - *Process-held state:* "`Store.attach` rebuilds; fingerprint check" (DH L3319-3343). And "An in-memory vocabulary
    silently fell back after reopen: 'Storing data in memory... is dishonest'" (DH L2185-2189). The session-only
    counted exports are this trap: a new process silently took the slow route. Step 1 keeps the state in the store
    (the frames) and checks that it is the run's (the keeper's files against the kept part).
  - *§2.20 (d):* "Write paths must be O(inserted), not O(index)", and "Prove one-shot equals incremental". The read
    path here follows the first. The cross-verify switch is the second.
  - *"Persist running totals"* (DH L3345-3357): the document digests are running totals, kept with the frames.

## 5. Order of work

Each step is measured unprofiled on the 20-file corpus. Findings must equal the cold run's, after edits and
unchanged. Then the step is measured on the data layer's 9,010 files.

| Step | What | Measured cost it targets (0116) |
|---|---|---|
| 1 | Inputs from the changed documents; no replay of unchanged hits | Memo replay and lookup 1–2 s; 135 MB decoded; inputs hashed |
| 2 | Deltas as side structures of the relation (dead rows, appended rows); `__plus`/`__minus` views, not relations | Strata opened and indexed for deltas (part of 1.0–2.0 s) |
| 3 | The driver: rewrite per stratum when reached; base-program passes kept | Rewrite 0.5–1.7 s; whole-program passes about 1.8 s; strata split 0.4–2.0 s |
| 4 | Old rows read back only for strata reached | Kept part read 0.6–1.0 s; part of the 5.5 GB peak |

**The end target** stays 0090's: an edit of one function costs under 5% of a cold cross part. On the 20-file corpus,
the joins are about 0.7 s of an edit's 26 s of CPU today. The target is a run whose cost is mostly its joins.
