# 0108: The per-file memo is frames

Status: accepted (2026-11-10). It builds t32 stage 5.1 and 5.2, and 0104's move 3 for the per-file memo, on
this branch rather than More-ergonomic-rule-syntax, so that the work does not wait.

## Intro

**The problem.** The corpus runner keeps what a file's file-local part hands on: its rows of each exported
relation and of each base relation the cross part reads, and its output lines. That is the per-file memo. It
was kept in a store of its own: the facts engine's `facts_results` table, with a frame per (source, path, plan)
in a format of its own, rows that cascade from the source, and a plan set per program. 0103 §8 lists it as a
cache to move: its key is `local_key(d, h, p)`, a document, its content and the program, and its output is the
file-local part's rows for that document.

**What changes.** A memo entry is a frame in the store's frames, the store every relation the engine keeps uses
(t33 K1, K4):

- the **space** is the file-local program's (its plan: the build, the grammar, the program as lowered);
- the **part** is the entry's key, from the document's content and its path;
- the frame's **reads** name the document in full: its path with its document key, and the content's
  SHA-256 as four words. Its digest field holds the document key;
- the **rows** are the entry's: a row with its node count, the rows of each relation, and the output lines,
  in typed rows of integers.

A lookup reads the frames of the run's documents before the first wave, in one statement, and decodes each in
its file's wave. A write is queued behind the run (0102), as every frame is.

```text
space  = H("local", plan)                    // the file-local program: one space per plan
part   = H(content, path)                    // the content by its SHA-256, and the path
reads  = [(path, key), (sha256, h₀) … (sha256, h₃)]   // the document and its content, in full
frame  = (0, nodes, …)                       // the node count
         (2, r, width, n, …) (3, …name…)     // relation r: its width and rows' count, its name
         (4, r, v₁ … v_width)                // a row of relation r, its values as columns
         (5, o, …) (6, …) (7, o, c)          // output o: its name, its node columns
         (8, o, i, len) (9, o, i, …bytes…)   // output o's line i
```

## 1. Why one frame per entry

A frame per relation per document is the obvious form: each relation's rows kept as any relation's. It costs a
store row per relation per document: about 270,000 rows for the 9,010-file repository, against 9,010 entries.
The spike measured a row at about 94 B against about 5 B for a member inside a per-file blob, and found the same
illness again and again: a per-file cost turned into a per-row cost (prior work, below). A document's facts are
read together and written together, so they are one frame. Within it each relation's rows are typed rows, a
column per value, as the frames encode every relation.

Granules of documents (K4's form for the cross part's inputs) were the other choice. A granule holds many
documents, and a run reads its documents in waves, a window at a time. Each wave would decode whole granules, so
memory would hold every granule decoded by the first wave: the whole corpus's rows, against the 1 GB budget.

## 2. The text

Output lines and paths are text, and frames hold integers. The text is packed into the rows, eight bytes to a
value. It is not interned as atoms: the store never takes an atom back (0049), so every line of every finding
ever made would stay in every process's arena.

## 3. Content, not sources

The part is keyed by the content's SHA-256, not by the stored source. A content not stored yet (0104 step 1)
has its key at once, so its entry is written with the others, not after its source. A source's id is not part of
the key: ids are the store's own and may be given out again once collected, and no row cascades from a source.
The frame's reads hold the digest and the path in full, so a part that collides with another content's is a miss.

## 4. What collects an entry

- **Its plan's death.** A plan no kept manifest names is dead (0053); collection deletes the plan's space with
  its other rows.
- **Disuse.** Each run marks the parts it reads and writes as used, and parts no run of the plan used for
  `MEMO_RUNS` (32) runs are deleted. That is S134: an incremental load returns its dead rows. An edited file's
  old content, a deleted file and another worktree's content at the same path go this way. The plan's runs are
  every root's, so 32 runs leave room for worktrees that take turns.

The memo's plan digest moves to `rerail.corpus-local/v3`: the plans made before this decision die with their
`facts_results` rows, and no run reads those rows again.

## 5. What does not change

- The entry (`memo::Entry`), and every reader of it: the cross part, the statistics keeper, the SQL view.
- A session's memo: in memory, as before.
- `facts_results`, `facts_plan_sets` and `facts_completions` stay for their other users (the slice's memo, the
  planning choices, the prepared program, the matchers). `facts_only`'s pins do not move; the per-file memo's
  line in `cache_architecture` goes when those users move.

## Prior work consulted

- **ast-index-spike design index, §1 (L37):** "A row costs about 94 B. A set member inside a per-file blob
  costs about 5 B."
  - **Agrees.** One frame per document (§1).
- **History index, finding 6:** "The recurring illness: a per-file cost turned into a per-row cost."
  - **Agrees.** Same.
- **Design index, S134:** an incremental load must return its dead rows, or it fills the disk.
  - **Agrees.** §4's disuse collection.
- **History index, §2.16 (d):** key memoised analysis by what it is a function of.
  - **Agrees.** The part is the content, the path and the key; the space is the program (§3).
- **Decisions.** 0052 and 0053 (the memo, its plans and manifests), 0102 (the writer), 0103 §8 (the key),
  0104 (lookups before the first wave, a lookup writes nothing: the uses are marked by the writer, behind the
  run), t33 K1 and K4 (frames).
