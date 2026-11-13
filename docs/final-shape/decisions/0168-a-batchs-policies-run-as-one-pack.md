# 0168. A batch's policies of one grammar run as one pack

Status: built (2026-11-13). Applies decisions 0051 and 0089 to `run-policies`. Narrows O-batch-typing-once,
O-combine-hiding-single and O-prepared-decode to what is left of them. Numbered 0167 when first written;
0167 is django-bugs' record.

## Intro

`rerail-engine run-policies <root> <out> <policy.rr>...` runs a batch of policies over one tree. It ran them one
after another, each a whole run of its own. Each run read and lowered its policy with every library file the
policy includes, combined and hid the program, planned it, and evaluated it. The 44 data-layer policies include
mostly the same libraries. So a batch read, lowered and analysed those libraries 44 times, and evaluated each of
their relations once per policy that reads it.

The tree already has the shape that shares this work: a pack (decision 0051). Many programs are combined into one
program, each library module is held once (decision 0089), and the program is evaluated once. Every relation is
derived once, and every reader reads its rows. The `logic` command runs a directory of `.rr` files that way.
`run-policies` did not.

A profile of the per-policy batch on `dl1` (one Python file, 44 policies, cold, `RERAIL_TAPE_CACHE=off`) showed
that evaluation is about 5% of it. Most of the rest is setup that each policy repeats: reading and lowering `.rr`
files (11%), combining and hiding (8%), merging identical rules (6%), demand and inlining (8%), phase 0's
graphs (3%).

## Decision

`run_policies` groups a batch's policies by the grammar each one's run reads, and runs each group of two or more
as one pack. It then splits the findings back to their policies.

```
rerail-engine run-policies dl1 out a.rr b.rr c.rr ...
  # a.rr, c.rr, ...: `.language python`  -> one pack, out/pack-0
  # b.rr:           `.language rr`       -> alone, as before, out/1
  # each policy's findings -> out/<k>/rr_diagnostic.csv, in the order of the paths
```

- **The grouping reads only each policy's header.** That is its directives and declarations, with no clause of
  its own or of its libraries (`rr::Text::header`). Lowering the headers costs a fraction of lowering the rules,
  and the pack lowers the rules once anyway. A first build lowered each policy's own rules for grouping
  (`rr::parse_own`): that cost 0.84 s of the batch's 15 s. A batch with the tape cache on lowers nothing at all,
  because its combined program comes from the store (decision 0094 §6).
- **The group key is the grammar the run reads** (`corpus::run_language`): the policy's `.language`, or, without
  one, the grammar the tree's files give (`corpus_language`). `leg-083-typed.rr` names no language and reads
  Python, so it joins the Python pack.
- **Each finding is told apart by its policy's id** (`.policy "id"`): every `rr_diagnostic` row names it. A group
  holds each id once, and holds at most one policy without an id.
- **These policies run alone, as before:**
  - a policy whose header does not read (it then fails with its own error);
  - a policy whose id another policy of its grammar already has;
  - a policy that declares no findings;
  - a policy whose language has several dialects (TypeScript), since each dialect is its own run
    (`run_dialects`).
- **Failures:** if a pack's run fails, or a finding names no member's id, the group's policies run alone. Each
  then fails or succeeds with its own error, as before.
- **The report is unchanged in shape.** `diagnostics` lists the findings in the order of the paths. Each policy
  in `runs` keeps its `path` and `metadata`, and gains `together`: how many policies its run held. `files` is the
  pack's count of files read.
- **`metadata` comes from the header.** It differs from a run alone's only in the `.global library` settings,
  which the reader writes for each file a policy includes (decision 0091). Those are the engine's settings, not the
  policy's.
- **`RERAIL_BATCH_PACK=off`** runs each policy alone, to compare. This is a measurement switch, like
  `RERAIL_LIBRARIES=copied`.

## Measured

On `dl1` (44 strict data-layer policies, CPU, debug, `RERAIL_TAPE_CACHE=off`, `RERAIL_THREADS=4`):

| | instructions | user CPU | wall | load |
|---|---|---|---|---|
| each policy alone | 147.9 G | 14.2 s | 27.4 s | 54 |
| one pack, rules lowered to group (first build) | 141.8 G | 13.1 s | 28.0 s | 34–64 |
| one pack, headers only | 137.5 G | 11.3 s | 19.8 s | 23–39 |
| `logic` over the same 44 files (the pack alone) | 134.9–136.1 G | 12.9–13.5 s | 18–21 s | 23–60 |

With the tape cache on (the normal case), each mode over its own store, run twice (load 47–63):

| | first batch (fills the store) | second batch (warm) |
|---|---|---|
| each policy alone (`RERAIL_BATCH_PACK=off`) | 187.1 G | 89.5 G |
| one pack | 170.2 G | 71.1 G |

Third and fourth batches over the pack's store took 74.3 G and 71.2 G, with the same findings. A third batch over
one store is where keeping a single program's hidden form failed before (O-combine-hiding-single).

- **Findings:** 48, the same rows in the same order. Metadata is the same for all 44, apart from
  `.global library`.
- **What the pack still costs.** The pack's 4,287 rules (from about 30,000 pasted ones) are not proportionally
  cheaper, so the saving is 7%, not the 50% the per-policy profile suggested. In the pack:
  - `combine` is 46 G of 134 G: parsing 2.9 s and merging the programs 1.4 s;
  - `prepare` is 52 G: prune, simplify and demand 1.2 s; pushing the file-local joins into the chunks 1.4 s,
    which splits the program 8 times; merging identical relations again 0.6 s;
  - inline-or-store is 1.0 s on the local part and 0.4 s on the cross part.

  These passes run once now, over the whole pack. Their cost is the next lever (O-pack-passes).

## Loading the pack, made lighter

Three passes over the combined program did more than their job needed. A profile of the pack's `combine` showed:
- **hiding** (`corpus::hiding::hide`) at 3,046 of 6,719 samples;
- **merging identical relations** (`simplify::merge`) at 1,392;
- **guarding the fact spaces** (`scoping::guard`) at 1,089.

What changed (the programs they produce are the same):
- **Relations alike but for their constants are found only when profiled.** `merge` made a second congruence over
  the whole program, with constants ignored, only to report relations one keyed relation could serve. That report
  is a profile's note (`relations equal up to their constants`), so `simplify` makes it only when profiled
  (`similar_relations`). It was about half of each merge, including the merge `prepare` runs again.
- **Hiding reads the rules in place.** Each round recorded its edits (a literal's kindset, or its relation renamed
  to a copy) against the unchanged rules and applied them after the round. It no longer cloned every rule each round
  as a snapshot. Its maps of carried and read variables borrow the rules' names instead of copying them.
- **The guard takes the rules out and puts them back**, instead of cloning them.

Checked: a temporary harness ran the old and the new `hide` side by side on all 42 calls of the dl1 batch, pack
and per-policy, and their rules, declarations and kindsets printed the same. Findings: 48, the same.

Measured on dl1 (44 policies, one pack, debug, `RERAIL_THREADS=4`, load 62–87), against the same tree before:

| | before | after |
|---|---|---|
| cold, `RERAIL_TAPE_CACHE=off` | 137.3 G | 124.8 G (12.8 s user) |
| first batch over a new store | 170.0 G | 158.3 G |
| second batch (warm) | 69.9 G | 63.7 G (6.6 s user) |

Spans in the pack (cold): hiding 0.46 → 0.35 s; merging identical relations 0.39 → 0.24 s; merging identical
relations again in `prepare` 0.58 → 0.28 s. The fact spaces (0.41 s, a full `split` of 2,134 relations for the
locations alone) and parsing the 44 programs (0.90 s) are what is left of loading (O-pack-passes).

## Tests

`rr::tape_tests::a_batchs_policies_of_one_language_run_as_one_pack` runs four policies over one tree:
- two Python policies sharing `python/calls.rr`;
- a policy with neither `.policy` nor `.language`;
- an `rr` policy.

It checks:
- the findings are each policy's own run's, in the order of the paths;
- the three Python-tree policies were one run (`together` 3) and the `rr` policy its own (1);
- each `out/<k>/rr_diagnostic.csv` holds only that policy's findings;
- each policy's metadata is its run alone's, less `.global library`.

Under `RERAIL_BATCH_PACK=off` the test fails on `together`.
