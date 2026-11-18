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

Found along the way: `a_maintained_run_decodes_and_copies_no_tree` failed on the committed code: `decode_kept` read
the lowered program's tree for its inputs and declarations. It now reads the version's fields.

## A reach index's delta (2026-11-17)

Each later round still derived one stratum again: `chain_root (1 relations, reads chain_below)`, "a changed read's
old rows not kept". `chain_below` is a transitive closure, held as a reach index (0119). A native stratum has no
maintained form: the driven walk made it again from its reads, as the base program does, with no delta, so its
readers derived again. A reach index cannot be made inside a form (the engine builds one in place, on the base walk).

Now, where its old rows are kept in memory, it is still made in place, and its delta is then made against them
(`reach_delta`: both row sets sorted and compared, held as `chain_below__minus` and `__plus`). Its readers are
maintained. `retain::after` keeps an index's relation when the part stays in memory (`indexes`: the guest rounds,
no store); a session's and the cone's still do not keep it (0119 §3: it is made again in less than reading it back
costs), so their path is unchanged. Where the index is not made in place (`reach_rows`), its readers derive again
as before.

M3, profiled: every later round 0 derived again, no relation without a delta (was 1 and `chain_below`). The two
strata cost about 0.05 s a round, so the cold CPU did not move outside the machine's noise (under load 40–85:
58.2 s before, 55.0 s after). Outputs identical (660 relations, 30,449 rows).

## After the hub merge (968fa0dc6, 2026-11-17)

The hub takes the embed slice from the prepared program's version (`embed_slice` over `prepared_placed.v`), not
the lowered tree: 584 rules, against 488. Its demand relations for aggregates (`rr_demand_agg_*`) are in it.
Measured back to back on the M3 (load about 12, one sample each; outputs identical, 660 relations, 30,449 rows):

| | cold CPU | slice | guests (profiled) |
|---|---|---|---|
| before this decision (t9) | 60.0 s | 488 rules | 11.4 s |
| the hub alone | 57.8 s | 584 rules | 11.6 s |
| the hub with this decision | 62.6 s | 584 rules | 11.7 s |

Over the larger slice the maintained rounds no longer gain: rounds 2 and 3 take 5.3 s (3.4 s before the merge),
reaching 173–219 of 256 strata, every one maintained or priced into deriving again (0090 §4: `rr_agg266`,
`type_ctx`, `ann_root`, `rr_demand_agg_rr_agg341`, `wstart` the largest, none over 0.2 s). The 4.8 s between the
last two rows is outside the guests (their spans differ by 0.1 s): one sample's noise.

### Why the slice grew (investigated 2026-11-17)

Not the cut's source: the old slice was cut from the prepared tree too, and the hub's commit (3d7bb7c6a, 0281 step 4)
only reads the same program from its image. The rule files are the same. What grew is the cone below the roots:
`RERAIL_DUMP_SLICE` (each relation with the rule that read it first, and the slice's rules) shows

    value <- dunder_all_at <- dunder_all <- public_name <- exports <- resolve_export <- sym_of <- typing_sym
          <- type_root <- type_ctx <- rr_ctx349 <- rr_embed_req

`dunder_all` reads `value(s, x)` (the library's `imports.rr`). `value` is answered on request (0174), and its
request relation `rr_value_req` is one relation for the whole program: 46 rules, one per reading site anywhere
(Ruff's C4xx, PYI0xx, Q011 and version checks among them), never inlined since 0291 (item 3). Following reads, the
slice takes all 46 and what their bodies read: `call_kwarg`, `c4_*`, `bytes_lit`, `cmp_pair`, `pos_item`, their
demand splits (`pre_*`) and narrowing asks (`rr_value_req_by_*`). 108 strata new against t9, 3.6 s of the guests'
profiled CPU; the maintained rounds reach them too.

`front::prune` already knows which request rule serves which reader (`Program::asked_by`, the `asked` about rows)
and keeps a request rule only while one of its readers is reached. The slice can be cut the same way, but the
record is by position among the relation's rules, and only lowering keeps it: the prepared program has 46
`rr_value_req` rules against 36 recorded readers (`rr_token_req` 5 against 3, `rr_node_text_req` 4 against 1),
since the passes after lowering (simplification, the preparation pipeline: joins pushed, rows located, repeats
dropped) rewrite request rules and leave the record. With a record that does not match, a gate must fall back to
taking every rule (`prune` does), so a gated slice cut (`working-data/patches/slice-request-gate.patch`) changes
nothing today. A rule's derivation source (`R::From`) does not survive the store's fold of an edit into a base.

Chosen (2026-11-17), of (a) the passes after lowering keep the record, or (b) the slice cut from the lowered
program and prepared on its own: (a).

### The record carried (2026-11-17)

Traced pass by pass, the record broke in the tree before the version was built, not in the version's passes:
`requests_file_local` split request rules into narrowed bodies and merged equal ones (36 to 41 `rr_value_req`
rules), `simplify`'s filter inlining copied a rule per definition (36 to 39), `long_text_twins` added a twin beside
a rule (39 to 40), hiding appended seeded copies, and `drop_repeated_rules` dropped repeats. Each such pass now
states where each rule came from: `Program::set_rules_from` (with `carry` and `set_rules_carried` for a pass that
takes the rules apart first) rebuilds the record, a rule serving the readers of the rules it came from together;
`push_rule_from` gives an appended copy its source's readers. A rule of no known source serves its own relation,
which `prune` keeps wherever the relation is needed: the record is always complete, and a pass that forgets it
ungates one rule, not a relation. The version build states a relation whose rules no longer match as a decision
(`the request record`).

With the record whole, `prune` gates the whole program's requests (`rr_token_req` 5 rules to 2), and the embed
slice gates them as `prune` does (`embed_slice`, `Version::asked`). On the M3 (cold, 4 threads; guests from the
profiled run):

| | t9 (before the hub) | merged | the record carried |
|---|---|---|---|
| embed slice rules | 488 | 584 | 491 |
| guests, s | 10.40 | 12.62 | 9.90 |
| run CPU, s | 58.2 | 59.7 | 52.5 |

Outputs identical in all three (660 relations, 30,449 rows). The maintained rounds' gain is back: round 2 3.05 s
against 5.08 s merged, the slice's first round 2.00 s against 2.48 s. One sample each; the M3's load was 11–24.

### Reviewed: readers renamed, folded, copied or gone (2026-11-17)

The gate names readers by relation, and passes after lowering rename them: `simplify::merge_identical` merges a
class into one name, hiding's `fold_own` folds a guest's copy back into its original, and inlining removes a
reader. A request whose readers were all renamed away could never open its gate, and would be dropped while its
reader's new name needed it. Now `Program::rename_readers` renames them in both merging passes, a copy under
another name joins its source's sets (`push_rule_from`), and a reader the program no longer has keeps its requests
in `prune` and the embed slice (it cannot be asked). With the renames, no recorded reader is gone at the version
build (`RERAIL_DUMP_SLICE`'s `.readers`), and the slice stays 491 rules; with only the gone-reader rule, before the
renames, it was 520.

Confirmed on the M3 (load 3.5 to 5), three samples each, engines interleaved: cold, warm (again on its store) and
an edit (one line added to one file). Medians, CPU:

| | t9 (before the hub) | merged | the record carried |
|---|---|---|---|
| cold, s | 55.5 (54.9–56.5) | 59.1 (52.8–59.6) | 52.0 (50.0–55.5) |
| edit, s | 5.42 | 3.92 | 3.90 |
| warm, s | 0.23 | 0.11 | 0.11 |

Outputs identical, cold and edit (660 relations, 30,449 rows). Four runs' wall stalled 46 to 147 s at their usual
CPU, across engines, while the host's connection was dropping: the host's, not an engine's.

### The record kept through every pass (Q50, 2026-11-17)

Working Q50 (the red `dropping_unreached` family) found that the record still broke in four places, each now
fixed: `plan::lower`'s `expand_memberships` split a rule per set member without its source; `prune` kept readers
it had dropped, so a later gate took them for readers gone by a rename; the pass manager's steps did not carry the
record (60 entries against 56 rules after `drop repeated rules`, 12 against 13 after the pushed joins); and the
version's merge did not rename readers as the tree's does. The pipeline now carries it by rule id
(`AskedByRule`: a rule kept, made from a source, or equal to a rule the step removed keeps its readers; any other
serves its own relation) and restates it at its end; a part keeps the record of the relations whose rules it
holds; a debug build asserts an entry per request rule where the image is made. The narrowing (0291) is per
reader site now: a narrowed request serves its askers, and only the sites that cannot narrow get a wide request.

On the M3, three samples each, interleaved (load 8 to 69, sample 1 of the new engine under the highest). Medians,
CPU:

| | t9 (before the hub) | the record carried (19ecd0b18) | kept through every pass |
|---|---|---|---|
| cold, s | 59.7 | 54.1 | 51.2 (50.6–63.6) |
| edit, s | 5.56 | 4.04 | 4.03 |
| warm, s | 0.23 | 0.11 | 0.12 |

Outputs identical, cold and edit (660 relations, 30,449 rows); the embed slice 491 rules, 285 relations.
