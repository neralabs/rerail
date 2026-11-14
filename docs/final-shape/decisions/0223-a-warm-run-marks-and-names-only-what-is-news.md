# 0223: A warm run marks and names only what is news, and keeps no text it only passes on

Status: built (2026-11-14). Follows decision 0211 (a warm run writes nothing) and 0213 (intern only what is stored).

## Intro

A warm run reads a store that already holds everything it computes. Decision 0211's check (`RERAIL_WARM`) fails every
write such a run makes. After 0212, PostHog's warm run wrote no derived frame, but it still queued about 70 writes a
run. Most were bookkeeping that the store's collection reads:

- **The memo's use marks.** Each part read was marked "used by this run", and each reading run counted itself in its
  space (`use_frame_parts`). Collection forgets a part that no run used in the last `MEMO_RUNS` runs.
- **The cross part's program uses.** Each run recorded `(program, run, relations used)` in `frame_program_used`, as a
  frames job with no rows. Collection keeps a relation that one of a program's last `RUNS` uses names, as long as that
  use is less than `ABANDON` runs old.
- **The manifest's late sources.** The combined program's text, its program's text and the planning choices' text
  were each looked up on the store's writer, so that each would find a source that an earlier queued write stores. A
  lookup there is a write: it pins what it finds.
- **The run's atoms**, flushed at the end of each run even when none were pending.

None of these changes what collection keeps when the store already says it. Each is now written only when it does.

A different cost was in the interpreter. The findings' JSON is one text made of about thirty pieces, joined two at a
time (`rr/lower/findings.rs`). Every partial text was interned as an atom, and stored.

## Decision

**A use mark is written when the part is aging.** A frame read from the store carries its age: the runs its space has
counted since a run last marked it (`Admitted::age`, read beside the frame: `frame_spaces.runs - frame_part_used.used`).
A reader marks a part only once it is `keep / 2` runs old (`Admitted::ages`). Marking also counts the run, so a warm run
after a counted one finds every part young and writes nothing. A part read at least once every `keep / 2` counted runs
is never forgotten. Before, the bound was every `keep` runs. Writing runs still count themselves and mark what they
write, so the count still advances and unused parts are still collected.

```
let ages = admitted.ages(MEMO_RUNS);            // memo::by_digest_from
if ages { used.push(part); }                    // only aging parts are marked
```

**A program's use alone is written when it is news** (`ContentStore::use_is_news`). The snapshot that is read before
the first wave now also holds each space's count of runs and each program's newest use (`Snapshot::uses`). That is
memory ahead of SQLite, with no SQL inside the waves. A use queued with nothing else to write is dropped unless the
newest use misses one of its relations, or is `ABANDON / 2` runs old. Each queued use updates the snapshot
(`note_use`), counted as `count_run` counts it. A use queued with frames, deltas or removals is always written: that
run counts itself.

**A late source that is stored already is named at once.** `memo::named_source` and `combined::name_program` read the
store before the waves. A source found there goes into the manifest's list at once, and only its pin is queued. A pin
is a lock, which 0211's list allows. A source that is not found is looked up on the writer, as before, only when a
queued write may still store it (`ContentStore::queued`, the combined entry's write key). With nothing queued, the
writer would find nothing either.

**A text read only as text is held, not interned** (`setfix::TEXT_OUT`). Sometimes every later reader of a text is a
`cat`'s part or a text function's text argument. Then the interpreter keeps the text in the branch's scratch
(`Ctx::texts`, a value `SCRATCH + i`) and drops it when the branch returns. The device already holds such a text in its
arena. Only the text a head keeps, or a step looks up, becomes an atom.

**The run's atoms are flushed only when some are pending.**

## Consequences

- PostHog after a forked cold run: the second warm run writes nothing to the content store but "the kept plans" (see
  below), with 29,747 findings as before. The one-file corpus's second warm run writes nothing to the store at all.
- One-file store: 1,777 atoms and 38 KB of strings (0213 left it at 1,984 and 57 KB). Findings, diagnostics and the
  other outputs are identical.
- The first warm run after a cold run still writes twice, once:
  - **Planning choices for the residual program.** A warm run binds the kept strata and plans the program that is
    left. A cold run never plans that program, so the warm run makes its choices and stores them. The next warm run
    finds them.
  - **A new manifest.** A cold run names the tree counts it measured. A warm run reads none and names none.
- Not this decision's (the hub's): a warm run with the cutoff off runs its joins, so it measures them.
  - The cost samples are committed to `costs.sqlite` (0100 §14, 0202).
  - Its join notes mark the join memo dirty (0203 §6), and "the kept plans" are written again with them.

  A rule like the one above would write a note only when it changes a decision the planner reads.
- Tests:
  - `frames::tests::a_part_read_is_marked_only_once_it_ages`;
  - `frames::tests::a_use_that_repeats_a_young_one_is_no_news`.
