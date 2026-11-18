# 0331. A chunk's guest rounds are one maintained program, driven by its deltas

Status: decided (2026-11-16). Builds on 0055 (guests made in rounds), 0145 (`maintained::run`, the cone's rounds),
0117 step 3 (a session's run walks the base program, each stratum's form made when a delta reaches it), 0077 and
O125 (kept rows are whole relations), 0094 §4, 0096 step 4 and 0123 (a kept relation's statistics kept with it).

## Intro

A chunk's files can ask for *guests*: an annotation's text parsed as an expression, a string's contents parsed as
code. The embed slice of the program runs over the chunk, the guests it asks for are parsed, the batch is built again
with them, and the slice runs again, up to four rounds (0055). Round 1 sees the files alone. Round 2 adds most guests
(the annotations). Round 3 adds a few (nested strings).

Each round ran the whole slice from nothing over the whole batch. Here the rounds are one maintained program: a round
keeps what it derived, and the next runs from its inputs' new rows only. A batch only gains documents between rounds,
so every delta is rows added, none gone.

## What made the first version slower than running each round from scratch

The first version used `maintained::run` as the cone does. It cost 57.1 s cold CPU against 55.8 s. Four causes,
each measured on the M3 (7 chunks, 20 slice runs, profiled):

1. **Statistics from round 1.** The slice's statistics were measured in round 1, when the guest relations are
   empty, and reused. Priced at no rows, `parsed($s, "annotation", $r)` was joined straight after `ann_input`'s delta,
   before `ann_strip(t, s)` binds `$s`: 1,942 rows to 7.5 M, all rejected two steps later (`ann_input`: 30.9 M units
   of work, against 1.3 M). The old engine had the same stale zero, but priced `ann_strip` at one row and joined it
   first by luck. Now each round's statistics are scaled to its rows, as the run's are, and a relation measured empty
   is measured again once it has rows.
2. **The whole program rewritten each round.** `maintained::run` wrote the maintained form of all ~1,000 strata
   every round, and set each up, for the few a round's guests reach: 1.1 s in the rewrite, 0.9 s measuring kept
   relations again, about 0.5 s of per-stratum setup. It now drives its run as a session does (0117 step 3): the
   cut's base program, each stratum's form made when a changed read reaches it, a quiet stratum's rows held as
   kept. `driven_part` (the base program with the given deltas) and `kept_statistics` (what the next run keeps of
   the bound relations' statistics) are the session's code, now shared, and every kept relation is bound through
   `bind_kept`, so its statistics stay with it while it is unchanged.
3. **Demand-cut rows kept as whole ones.** `maintained::run` did not hold `demand::whole_relations()`, which a
   session holds while it evaluates. The planner's demand split computed `demand_*` relations and their readers for
   the keys asked for only. Those relations were of no stratum the run kept, so each later round derived about 100
   strata again (no delta for `demand_*`, then none for their readers). And rows computed for some keys were kept
   and maintained as if whole: the O125 fault the guard prevents. The guard is now held for every maintained run.
4. **Deltas by hashing.** Each round rebuilds the batch, so an old source is rarely a prefix of its new one, and
   `grown_delta` hashed every row of both: 0.76 s. The caller knows the delta by construction: the rows whose node
   column is of a document made this round (`owners_delta`), checked by count (the old rows with those are the rows
   now), else `grown_delta`. 0.12 s.

Not a cause: the pricing of "derive again or maintain" (0090 §4). It runs only past θ with at least 1,024 delta rows,
and chooses both ways (`ann_input` derived again at less than half the maintained price; `ann_ctx`, `ast_name`
maintained).

## Outcome

M3, cold, 4 threads, the full shape (317 rules, corpus0):

| | before (t9) | now |
|---|---|---|
| cold CPU | 55.6 s | 53.0 s |
| guests (profiled) | 10.6 s | 8.9 s |
| rounds 2 and 3 (profiled) | 5.7 s | 3.4 s |
| strata derived again per later round | ~100 | 0–1 |

Outputs are identical (660 relations, 30,449 rows).

## Open

- `chain_below` is derived again in the later rounds: a read of it has no delta.
- `a_maintained_run_decodes_and_copies_no_tree` failed on the committed code: `decode_kept` read the lowered
  program's tree for its inputs and declarations. It now reads the version's fields.
