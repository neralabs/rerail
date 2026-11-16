# 0227: Frames are kept while a kept manifest reaches them, not by how recently a run used them

Status: built (2026-11-14). Replaces the disuse rule of decision 0146 §4 (`MEMO_RUNS`) and the run-counted ageing of
maintained frames in decisions 0117 ("A run's use is one row") and 0138 (`frame_part_used` as each frame's last
run). Follows decision 0211, whose warm runs it lets write nothing.

## Intro

The store keeps two kinds of frames:

- **memo entries**: one file's results under a plan, keyed by the file's content (decision 0146);
- **maintained frames**: a maintained part's relations (the cross part, the cone's rounds), kept between runs so
  the next run derives only what changed (decision 0117).

Until now both were kept by recency. Each run marked what it used, and a frame no run had used for `RUNS` (4) or
`MEMO_RUNS` (32) runs was deleted. So every run wrote, even when nothing had changed: the marks were most of what a
warm run with the cutoff off still wrote (decision 0211, Open). And a frame was kept for as long as runs happened
to touch it, not for as long as something could read it.

Now a frame is kept while a kept manifest **reaches** it:

```
kept manifest ─ names ─▶ live plan / program ─ owns ─▶ its space's frames of its relations
      │
      └─ its sources ─▶ their contents' sha256 ─ key ─▶ memo entries in a live plan's space
```

Collection deletes the following. It runs as the `frames <store>` upkeep job (`frames::collect_unreachable_frames`,
by `ContentStore::sweep_frames`): a utility job (decision 0190 §7), due once a keep decision is past the last sweep (`corpus_decided.swept`, a column added on open: no table of its own, decision 0104),
run soon after it behind the run, and finished before the run's process exits.

| What | Unreachable when |
|---|---|
| A memo entry, in a live plan's `local_space` | its content's sha256 is not that of any source a kept manifest holds (`corpus_live`) |
| A maintained frame | its relation (`used_key`) is not among the relations of a live program in its space (`frame_program_used`, run 0) |
| A `frame_program_used` row | its program is no live plan, or it is a per-run row from before this decision |

A run marks nothing. It writes a program's relations (`frame_program_used`, run 0) only when they differ from those
stored, and stamps its puts as before. A part with nothing put or forgotten queues no write (`Kept::flush`,
`kept_strata::keep`, `Retained::write_frames`).

## The manifest names every part it keeps

Reachability is only as good as the names. A manifest must name every program whose frames the run keeps:

- the prepared program (`prepared_key`), whose space holds the cross part's strata (`Kept`);
- the cone's rounds (`retain::key(&[0; 32], round)`);
- the cross part (`retain::key(&prepared_key, cross)`), named in `part_keys` as it runs.

The last one is known only once the cross part runs. A run with no cross-file memo plan used to record its manifest
before that point, so its cross part was named by no manifest. That was true of every phase's run and of phase 0's.
Phase 0's cross part (18 `p0_*` relations, 67 frames) was then collected after every cold run, by that run's own
pass, and derived again the next time. The `sources` phase's part survived only because the outer run happened to
name it.

Now a run with a durable store records its manifest after its cross part, as one with a cross-file memo plan
already did (decision 0076). The lease on its sources is held until then, as decision 0053 allows. The collection
report counts the programs with rows that no manifest names (`unnamed`), so a part that is missed again shows there.

## Measured

On `tmp/warm` (dl10, `layer-violations.rr`, and the 44-policy batch over dl1g), at load 50 to 100:

| Run | Before | After |
|---|---|---|
| Warm dl10 with the cutoff off: writes to the content store | use marks each run | none |
| Warm dl1g batch (`RERAIL_WARM=1`) | the marks written | nothing written, rc 0; 6.4 G instructions, the same findings (98c397…) |
| First collection on the old dl1g store | — | 33,927 maintained frames of dead programs and 66 per-run rows dropped, 0.393 s, once |
| Later collections | — | 0.001 to 0.009 s, nothing collected |
| Cold dl10 run after a rebuild | 67 frames of phase 0's cross part collected, every time | nothing collected; the next run reads them |
| dl10 edit, then revert | — | 5.5 G, then 4.9 G instructions (unchanged: 4.3 G): the reverted content hits the memo |

The memo sweep reads every memo entry's `reads` in each live plan's space. On the largest local store (120,169
frames, 390 MB of data) that full scan takes 0.42 s. It runs only after a keep decision, which is rare (decision 0053).
That was not worth optimising then. Decision 0253 had the sweep read each entry's digest from `frame_part_used` instead; since decision 0240 §4 the
entries are rows of `memo_entries`, each with its digest, and the sweep reads that table.

## What this gives up

- **A part no kept manifest names loses its frames at once**, where recency kept them for a few runs. That is the
  point: what no kept manifest names, no protected run can read. A missing name is now a cost seen on the next run,
  and the `unnamed` count shows it.
- **A memo entry of a content no kept manifest holds goes**, even if a run read it lately. An edit reverted after
  its manifest was evicted derives the file again. The protected runs (two a lane, 64 lanes a root) and the history
  kept by bytes cover the reverts measured.

## Open

- If the memo grows past what a full scan of its reads allows, `frame_part_used` can carry each entry's content
  digest, so the sweep reads that table alone.

Tests: `frames_are_kept_while_a_kept_manifest_reaches_them` and `a_keep_decision_leaves_its_sweep_to_the_frames_job`
(store). The recency tests went with the rule they tested.
