# 0450: Sessions keep their latest data live in segments; the rest ages out, and may be kept compressed

Status: decided in part by the user (2026-11-19), items 1–4 of §1. §2 (what a session is) is proposed, not agreed.
§3 (the cross-file memo) is surveyed, to be discussed. Follows 0295 (segments and the archive), 0053 (collection),
0353 (blocks are never copied) and the store's floor 7 (0295 step 5: no source or tape in a row).

## Intro

The store holds its bytes in three forms:

| Form | What uses it |
|---|---|
| **Segments**, raw, mapped, read in place | sources and tapes (all, since floor 7), the memo's entries (`.memo`, 0295 step 4), atoms (`.atoms`), kept facts (`.facts`, 0443) |
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

### 2. What a session is (proposed, not agreed)

A session is the store's one kind of owner: a row `(session, kind, key, latest generation, last used)`. Each of
today's owners becomes a session of its kind: a workspace (key: its root path), a corpus root and lane (0053), a
caller's session (0204), a pin's owner, an analysis scope. A session's latest generation names what it read and
wrote last; that set is live. Its older generations, and data no session's latest names, are aged: 0053 §3's history
budget and backstop become the one LRU and age rule for every kind.

Open: whether 0204's in-process session and the store's session are one thing; how a session ends (a lease, or its
caller); the budget's marks.

### 3. The cross-file memo (`cross_components`, `cross_blocks`, `cross_rows`), surveyed

- **What it is** (0076): a corpus run's rows of cross-file relations, memoised per Python import component and keyed
  by the component's descriptor digest (plan, key, documents' keys), so it is shared by content across roots.
  `cross_blocks` holds one blob per (component, relation): columns of zigzag varint differences. `cross_rows` is the
  older row-per-row table; this binary neither writes nor reads it.
- **Who uses it:** only a program that declares `.pragma "cross-memo"`. No shipped program does; only tests
  (`corpus/tests.rs`, `typing/oracle_tests.rs`, `findings/tests/rr_comparison.rs`).
- **What it measured** (0076 §7–9, Jedi corpus): a warm run 53.7 s → 13.2 s; one file edited, it does not pay (cross
  part 42.2 s against 29.9 s with the memo off).
- **Its format** is decoded into in-memory columns on read: a copy of a block (0353), and not random access, unlike
  0443's packed chunks.
- **On the sibling branches:** phased-facts retired the frames and removed `encode_windows`, which only frames used;
  no branch extends the cross memo. 0083 keeps it "for programs that declare them".
- **Kept facts** (0438, 0443) serve the edit case: a run binds the relations kept by the previous one where they lie
  and derives only what changed. They are kept per space (a program's part). Whether a space is shared across roots,
  as the cross memo is by content, is not checked here.

## Order of work

1. Agree §2 and §3.
2. The live tier for the zstd rows: results, call summaries and tree pages placed in segments, rows pointing at them
   (as sources and tapes, floor 7). The facts memo waits for §1 item 4.
3. Sessions as the one owner, with the latest generation live and the LRU and age rule (0053 §3 generalized).
4. The archive: aged data moved to zstd rows by upkeep, segments rewritten (§1 item 2), and placed again in a segment
   when read (§1 item 3).
