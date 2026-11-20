# 0450: Sessions keep their latest data live in segments; the rest ages out, and may be kept compressed

Status: decided by the user (2026-11-19): items 1–4 of §1, §2 (two sessions, how a store session ends, and its
shape: "go ahead with 0450"). Step 2 built (floor 8); steps 3 and 4 built. §3 (the cross-file memo): its history traced; its removal
decided by the user (2026-11-19, "go ahead with that split") and done. Follows 0295 (segments and the archive), 0053 (collection),
0353 (blocks are never copied) and the store's floor 7 (0295 step 5: no source or tape in a row).

## Intro

The store holds its bytes in three forms:

| Form | What uses it |
|---|---|
| **Segments**, raw, mapped, read in place | sources and tapes (all, since floor 7), the memo's entries (`.memo`, 0295 step 4), atoms (`.atoms`), kept facts (`.facts`, 0456) |
| **zstd in a row** | per-file check results and call summaries (`local_check_results`, `context_check_results`), the facts memo (`facts_results`, `facts_completions.counts`), per-directory result pages (`tree_pages`) |
| **Raw blob in a row** | `cross_blocks`, `compiled_plans.program`, `accel_models.archive`, `corpus_manifests.sources`, prefilter and term postings |

The zstd rows are read on every restore or run (decompressed each time, as tapes were before floor 7). On the bench
stores the facts memo is the largest of them: 3,763 rows, 20.7 MB raw kept as 4.3 MB.

What keeps data alive is decided per owner, each with a rule of its own: a workspace's lease, a corpus root's last 2
runs plus a history under a byte budget and a 30-day backstop (0053 §3), an object pin's lease, a snapshot, a kept
facts space (`facts_chain`), and on git-commit-tree-metadata the analysis scopes and audit snapshots. Rows die with
their source when no owner names it.

## Decision

### 1. Decided by the user (2026-11-19)

1. **Long-term is by owner, and the owner is a session.** The latest data of each session stays alive. The rest is
   handled by LRU or by ageing out.
2. **A segment may be rewritten** when it holds data partly kept and partly not: the kept part is copied to a new
   segment and the old one removed (as `compact_object_segments` does for sources and tapes).
3. **Data read again goes back to a segment.** The engine reads in place, so data kept compressed in a row is
   placed in a segment before a run reads it.
4. **The facts memo against kept facts** is being straightened out elsewhere: not decided here.

So the tiers are:

| Tier | What | Where |
|---|---|---|
| live | every session's latest data | segments, raw, read in place |
| aged | data no session's latest names, inside the LRU or age budget | SQLite rows, zstd (the archive, 0295 §4), or segments until archived |
| gone | past the budget | collected |

### 2. Two sessions (the user, 2026-11-19: "I think we need two sessions"; validated below)

**The engine session** (0063, 0077, 0204: `logic/plan/corpus/session.rs`, `session::Handle`) is what a long-lived
caller keeps from one run to the next so that a run after an edit costs what the edit changed: the file-local memo
held in memory, the cross-file part's retained strata (0077), the outputs and stacks it wrote, kept statistics, open
connections. It is a value: its caller owns it (the browser's engine, `repeat` through `RERAIL_SESSION`, a test), it
is numbered by a process counter (`GENERATION`), and it ends when its caller drops it. What it holds is memory ahead
of the store, never what keeps the store's data alive. It opens a store of its own, `session.sqlite` in its
directory.

**The store session** (this decision) is a durable owner in the store: what keeps data alive between processes and
after them. Its key is a durable name (a workspace's root, a corpus root and lane, a pin's owner, an analysis
scope), and its latest data is live (§1).

They differ in every respect that matters here, so they are two things:

| | engine session | store session |
|---|---|---|
| lives in | a process's memory | the store's rows |
| identity | a process counter | a durable key |
| ends | when its caller drops it | when nothing keeps it alive |
| holds | data ahead of the store, for speed | which data the store keeps |

The link: a bound engine session is one of the things that keeps a store session alive, and what it writes durably
(memo entries, kept facts, results) is the latest data of that store session.

**A store session ends when nothing keeps it alive any more** (the user, 2026-11-19). What keeps one alive: an
engine session or process bound to it (shown live as a process's lock is), a lease (a workspace's), a pin, or a
durable name that refers to it (a branch, an audit snapshot). With none left, its latest data is no longer live: it
is aged like the rest (§1), not deleted at once.

**Agreed (the user, 2026-11-19: "go ahead with 0450"):** a row `(session, kind, key, latest generation, last used)` for each store session. Today's
owners each become one: a workspace, a corpus root and lane (0053), a pin's owner, an analysis scope. 0053 §3's
history budget and backstop become the one LRU and age rule. Open: the budget's marks; whether the engine session
keeps a store of its own (`session.sqlite`) or binds a store session in the shared store.

### 3. The cross-file memo (`cross_components`, `cross_blocks`, `cross_rows`): its history

- **What it is** (0076): a corpus run's rows of cross-file relations, memoised per Python import component, keyed by
  the component's descriptor digest (plan, key, documents' keys), so shared by content across roots. `cross_blocks`
  holds one blob per (component, relation): columns of zigzag varint differences, decoded into columns on read (a
  copy, 0353). `cross_rows` is the older row-per-row table, which this binary neither writes nor reads.
- **The problem it was for.** The per-file memo (0052) let a warm run skip unchanged files, but the cross-file part
  ran whole every time: typing's took 12.6 s of CPU on the 84-file workspace (0059) and about 52 s on 489 files
  (O94). Designed in the Bend-language-investigation session on 2026-11-08 (00:57–01:14 UTC). The user asked "why are
  cross-file relations so special? why does the file matter? why can't they be memoized?", was offered a key per
  run, per import-closed group or per file's read set, and chose the group: "we need to do 2. Deriving the import
  chunks (the closure) and the key needs to be done via rerail and facts too, so that all of the work is memoized."
  Built in `cc5ca55c9` (2026-11-08 02:02 UTC, as "0063 steps 2–4").
- **Results** (Jedi corpus, 84 files with typeshed; 0076 §7–9):
  - over `calls.rr`: correct, not faster. Replay made the cross part 19.8 s against 16.2 s: only the cheap relations
    meet the contract; the costly ones look up every document by name under negation (O114).
  - over `calls_cx.rr` (generated from `calls.rr`, every lookup kept to the reader's import closure): a warm run 53.7
    s → 13.2 s; a one-file edit worse, its cross part 42.2 s against 29.9 s with the memo off. The key is the files'
    bytes, so there is no cutoff: one edit to a widely imported file missed 41 of 133 components.
- **When and why it stopped** (2026-11-08, about 15:00 UTC). Asked to survey the other worktrees' keying, the session
  found 0083 (Two-pass-roots-and-cross-file): phases, definitions as the unit of change, stored results maintained by
  deltas. It wrote "I'd stop extending the per-component memo here. 0083 replaces it for typing." 0094 (accepted
  2026-11-08) recorded "0076 is retired for typing", keeping its keyed one-query tables as a pattern; 0083 keeps "the
  mechanism ... for any program that declares it".
- **Since then:** no shipped program declares `.pragma "cross-memo"`. Only tests switch it on: `typing/oracle_tests.rs`
  through `RERAIL_PYTHON_CROSS_MEMO`, `logic/plan/corpus/tests.rs` and `findings/tests/rr_comparison.rs`. Every later
  commit touching `cross_memo.rs` carries it through refactors (versions, atoms, clippy). Kept facts (0438, 0456) now
  do what it did for the edit: a run binds the previous run's relations where they lie and derives only what changed.
- **Half of it is not the memo, and is about to be on the product path.** `components.rr` derives the import
  closure (`rr_cx_reach`, `rr_cx_sees`, the edges and components). `calls_cx.rr` and `calls_p3.rr` read it without
  the memo (`reads_components` in `corpus.rs`), and on phased-facts (`396f6f942`, 2026-11-18) the shipped
  `python/calls.rr` and `python/types.rr` include `points-to.rr`, which reads `rr_cx_reach` 29 times and
  `rr_cx_sees` 7 times.

**Decided (the user, 2026-11-19: "go ahead with that split"), and done.** The memo is removed: its lookup, record
and replay (`cross_memo.rs`), the `__cross` inputs and `rr_cross_doc` guards, the store's methods and its three
tables (`DROP TABLE IF EXISTS` in SCHEMA), `RERAIL_CROSS_MEMO`, `RERAIL_PYTHON_CROSS_MEMO` and `Report::cross`.
`.pragma "cross-memo"` is refused, as `cross-rounds` is. The import closure stays: `components.rr` is added to a
program that reads an `rr_cx_` relation (`import_closure.rs`). Its component key went with the memo
(`rr_cx_component`, and `rr_cx_own`'s digest of each tape: it is now one row per document). The delta-column
encoding the memo's blocks used is the frames' too, and stays (`content_store/delta_columns.rs`). The `components`
native (SCCs with a Merkle key) stays a language feature with no shipped user. This supersedes 0094 §7's "kept from
it: the `cross_rows` store" and 0083's "the mechanism stays for any program that declares it".
On final-rerail-shape-rr-align the frames are retired (0439), so with this merge the encoding had no user left and
went too.

## Order of work

1. Agree §2's store session. Done (2026-11-19), and §3's removal.
2. The live tier for the zstd rows: results, call summaries and tree pages placed in segments, rows pointing at them
   (as sources and tapes, floor 7). The facts memo waits for §1 item 4. **Built (2026-11-19, floor 8,
   `content-v8.sqlite`):**
   - `context_check_results` and `tree_pages` name a range of an object segment (`seg`, `seg_at`, `seg_len`); their
     bytes are as the caller made them, no zstd. A result is read where its segment holds it (`StoredLocalResult::
     bytes`, `TreePage::bytes`: `ObjectBytes::Mapped`); a page set is not hashed again on a read (the segment's check
     is the check).
   - A publication's relationship pass places its results in one segment written before its transaction, numbered in
     it and named after commit (`Placing`, as the object pass); `put_tree_pages` the same.
   - A result or page whose segment is gone is a miss (the file's results, the directory's pages computed again).
   - Compaction covers every placed table (`object_segments::PLACED`): sources and tapes marked absent when their
     segment is gone, results and pages removed; bytes several rows name copied once.
   - `local_check_results`, which nothing wrote, is gone.
3. Sessions as the one owner, with the latest generation live and the LRU and age rule (0053 §3 generalized).
   Planned in slices, each its own commit:
   - **A. Workspaces are store sessions.** A row of `store_sessions(session_id, kind, key, latest, last_used,
     ended)` per workspace (kind 1, key its root). Registration and a lease's renewal keep it alive (`last_used`
     now, `ended` NULL); `retire_workspace` or a lapsed lease ends it (`ended` then). Its overlay and snapshot are
     its latest data. An ended workspace's are aged, not dropped at the next pass as now: kept while the aged bytes
     are under the budget and it ended less than the backstop ago, the oldest-ended released first.
     **Built (2026-11-19):** `store_sessions(kind, key, latest, last_used, ended, aged_bytes)`, kind 1 keyed by the
     root's `path_id` (the row is the workspace's, so the id lives as long as it). A workspace's state is now 1 live,
     2 ended and aged, 3 released. Collection ends a lapsed workspace's session with its lease's end and the bytes
     (sources' and tapes' segment lengths) its snapshot and overlay name, counted once, at the end. It releases an
     ended one when it ended `backstop_secs` ago or the sessions ended after it, with it, hold more than `bytes`
     (`Ageing`, 384 MiB and 30 days, per store handle; a test's `set_session_ageing`). A workspace with no session
     row (an older binary's) is released at once. Registering over an ended workspace's root starts its session
     again and drops the old overlay, which was the ended session's: before, a revived row kept it. Test:
     `an_ended_workspace_is_aged_and_released_oldest_first`; the store's other tests release at once (their
     helper's `Ageing { bytes: 0, backstop_secs: 0 }`), as they pin collection's steps.
   - **B. Corpus lanes are store sessions** (kind 2, key root and lane): a lane's protected runs are its latest
     data, its older runs aged (0053 §3 as it is), under the same budget as A's.
     **Built (2026-11-19):** each keep decision rewrites the kind-2 rows from the manifests it keeps: one per (root,
     lane), keyed by the first eight bytes of a digest of both, `latest` its newest manifest, `last_used` its last
     run, and `ended` its last run when none of its runs is protected. The one budget: ended workspaces' aged bytes
     count first against the history's marks (`Aged`), and the sources and tapes they hold count for no manifest
     again, so shared bytes are counted once. A workspace is released by its own rule (A) at 384 MiB; the history
     takes what is left under 0053's marks. Tests: `ended_workspaces_aged_data_comes_first_under_the_one_budget`,
     `each_lane_keeps_its_last_runs_plans` (six lanes, six live sessions).
   - **C. A pin's owner is a store session** (kind 3): its lease keeps it alive.
     **Built (2026-11-19):** keyed by the owner's `strings` id; `pin_source` and `pin_tape` keep it live. Its session
     ends when its last pin lapses, with the segment bytes its source and tape pins name; the lapsed pins are then
     kept, and protect what they pin, until the session is released under the one budget with ended workspaces
     (`aged_sessions!`: kinds 1 and 3 ranked together by when they ended), and the corpus decision counts their bytes
     first as it does the workspaces'. Not aged: a live owner's lapsed pins (not its latest data); a plan's hold
     (`hold_corpus_plan`), whose lapse makes a keep decision due, so one kept would make every pass due; and an
     owner's explicit release (`release_pins`: a run's end, a gone process's holds), which ends its session with
     nothing aged. Test: `a_lapsed_owners_pins_are_aged_and_a_released_owners_are_not`. In production the rr images'
     pins (7-day leases) and combined programs' (6 hours, when no run released them) now age past their lease.
   - **The budget**, one for every kind: 0053 §3's marks (512 MB high, 384 MB low) and backstop (30 days), until
     the user sets others (§2 left the marks open).
4. The archive: aged data moved to zstd rows by upkeep, segments rewritten (§1 item 2), and placed again in a segment
   when read (§1 item 3).
   **Built (2026-11-19, `content_store/archive.rs`):**
   - **What is archived:** a source or tape in a segment that no session's latest data names: no live workspace's
     snapshot or overlay, no live pin, no `file_documents` row (tapes), and not a source of a corpus lane's latest run
     (its live session's `latest` manifest, or a run recorded since the last keep decision). It must also have an aged
     holder: an ended workspace, a lapsed owner's pin, or a kept corpus run (`corpus_live`). Garbage is left to
     collection. The latest data is the latest run, not 0053's two protected runs: the user's rule (§1 item 1).
     Results and pages are not archived: they are computed again, and compaction drops them with their segment.
   - **How:** the store's `Collect` upkeep job, after collection, while `corpus_state.archive_due` is set (a session's
     end in collection, or a keep decision, sets it; a pass that archives nothing clears it). A pass takes up to 256
     sources and 256 tapes, compresses each into one zstd frame outside the write transaction, then, in one
     transaction, marks each row `seg` -2 where it is unchanged and writes `object_archive(kind, id, frame)`. Its
     `seg_len` stays its length. Compaction copies only rows in a segment, so it then frees their bytes (§1 item 2).
   - **Read again:** every reader of sources and tapes by id or digest (`resident_frames`, `tape_frames_for`,
     `tape_frames_with_hashes`, `get_source`, `get_sources_by_hash_bulk`, `objects_bulk`, `get_tape`) places the
     archived rows it met back first (`placed_again`). Their frames are decompressed into one new segment (`Placing`),
     and in one transaction the rows are pointed at it and their archive rows removed. Then the reader reads again, in
     place (§1 item 3). So does a clean file's restore by its blob alias (`sources_by_alias_in_place`). A SQL module's
     reader, inside its statement on its own connection (the prefilter's `StoreTapes`, the pure-SQL `nera_tree`),
     places the tape back from a connection of its own (`place_archived_tape_at`) and reads it at its new place; it
     had failed on one ("a tape whose segment is not there"). A put of an archived object places the caller's bytes
     (`seg < 0`, as for an absent one). The one reader that does not place back is the search index's background
     builder: placing a tape back only to index it would undo the archive. An index made before the archive is kept,
     and a run that reads the tape places it back, after which it is indexed.
   - **Nothing a warm run reads is recomputed.** Its latest data is never archived, and archived data it meets is
     placed back, not made again. Results and pages are not archived, and they are not dropped for being aged
     either: they stay in their segments while their source (results) or name binding (pages) is kept, and
     compaction copies them. They go only with their source or binding, or if their segment file is gone.
   - **Pages put and not yet bound** (2026-11-19). A page derivation's name binding is written with its overlay
     layer, after the pages, and collection took a set no name bound: one put by a run whose layer was not yet written
     was collected, and its next lookup recomputed it. A put now writes `tree_pages_pending(digest, pid)` in its
     transaction. Collection keeps a pending set, drops the mark once a binding of its digest is written, and drops
     the mark of a process that is gone, which no binding will follow (`process_gone`, as for a run's holds). Test:
     `collection_keeps_pages_a_name_binds`.
   - **Archive rows** whose object went or was placed again are removed at the start of each archive pass.
   - Test: `aged_objects_are_archived_and_placed_back_in_a_segment_when_read`.
