# 0401. Labels start at their seeds; a matcher's captures are bound only where its readers join

Status: accepted (2026-11-18). Numbered 0396 where it was written.

## Intro

t45 measured where the matchers' work went after 0390 and proposed five changes, by payoff. The user asked for
all five (2026-11-18): "1. Check captures only at demanded nodes. 2. $X as a node test. 3. Label copies as bit
operations. 4. Launch label-domain passes only at members. 5. Start passes from a neighbor relation (item 4 is
needed first)." This records how each was done and what it changed.

Some terms first:
- A **label** is a node set that a pass fills before a matcher's root program reads it, as one bit per node.
- A **domain** is the set of nodes a pass starts from: all nodes, the nodes of some kinds, or a label's members.
- A **walk** is one start's run of the pass's program on the device, and a **step** is one instruction of it.
- A **capture** is a metavariable such as `$F` whose node a relation's row carries. The host binds it by running
  W's matcher again at each root (`ast_wave.rs` `captured_rows`).

## Decision

1. **A matcher's captures are bound only at the nodes its readers can join** (`matchers::demanded_by_roots`,
   `Matcher::demanded_by`).
   - Matcher A is restricted when two things hold. Every rule that reads A's relation joins A's node with a
     column of another positive atom. And that column's values come only from other matchers' roots. It is a
     matcher's own node column, or a derived relation's column that every rule of that relation binds from such
     a column.
   - A is then bound only at the nodes in those matchers' roots. The wave finds every matcher's roots before it
     binds any capture, so the intersection is known in time.
   - A is not restricted when its relation is an output, read under a negation or in an aggregate, or read by a
     rule with no such join. Inputs, natives, lattices and relations with facts never count as roots-only.
   - This is the atom's magic rule kept at node grain. `demand_program` widens it to the document, because the
     batch has not derived the body's relations yet. Here the body is the same wave's matchers, which have.
     internal_dead_code's `mn_function(w, f) :- raw_module_name(w, d, _), ast(…, $FUNCTION)` is bound only
     where `rr_ast_11` (the identifier rule `raw_module_name` comes from) matched.
2. **A bare `$X` is a node test.** A pattern that is one metavariable (`Program::bare_metavariable`) lowers to
   the named-node flag (`$$X`: nothing). It is no longer a pattern label, so it costs no pass and the walks that
   read it take no step for it.
3. **A clause whose program copies a label is a word operation** (`LoweredMatcher::label_copy`). The program
   is `SUPERTYPE label; CAPTURE; EMIT`.
   - On the host, its roots are the label's members in its domain.
   - On Metal, its pass is empty, and the decode reads the label's row, within the domain and the active files.
4. **A label-domain pass starts only at the label's members on the device.**
   - A label domain's program tests the label first, so the walk's anchor (decision 0365, `device_anchor`) holds
     the label: a derived class, which `narrowing` keeps. The anchor's row, the label's members within what else
     the walk tests at its start, is ANDed into each group's root mask (`root_mask_in`) and checked by a sibling
     scan's leader (`root_admits`).
   - Written first as a root kind of its own (`Roots::Codes::label`, `[2][row][file bits][rule]`, read by
     `root_mask_label`). The merge with the hub's 0365 anchors dropped it: the anchor is the general form, and one
     kind 2 is the root table's. A pass that is not a micro walk has no anchor, and starts at the label's kinds.
5. **An `all` that tests a neighbor starts at its seed** (`Builder::seed`, `Demand::seed`).
   - Take an `all` whose conjunct is `inside X` or `has X` by `neighbor`, through a field or not. It holds only
     at X's children, or at X's parents. Those nodes are a Targets label, the seed: a one-step walk from each
     node X matches. The conjunct becomes the seed's bit and is tested first.
   - The labels made inside that `all` start at the seed's members (`narrowed`). So do a count's starts, the
     `all`'s own roots label and a clause's domain. A neighbor walk alone, such as the rule inside a `not`, is
     its seed.
   - A seed is made only from a selective X, one with kinds or a label. From every node it would be one more
     pass and no fewer starts.
   - Narrowing to the seed is correct for the reason 0390's narrowing is: a label made in the `all` is read only
     where the `all` can hold.

## What the tests caught

The probe programs' rows were the same on the first build, but the Metal parity tests were not. Each of these
is fixed and has a test that hits it:
- **The root table's kind 2 was misread** (in the first form of item 4). `root_mask_in` and `root_admits` skipped
  the 2 and then tested the next word for 1, a listed rule. That word is the label's row, so a domain label at
  row 1 was read as a list. Different clauses failed from run to run, depending on which label came first. The
  anchors' kind 2 carries the offset of the rule past it, so the merged readers do not have this shape.
- **A pattern label waited for nothing.** `Labels::reads` left out a pattern's and a constrained pattern's
  domain, which had never been a label before item 5. A narrowed pattern label could be ordered beside its seed
  and read the seed's set empty.
- **A sibling scan left its work to an unlaunched leader.** A walk that is not its siblings' first leaves the
  scan to the first, where that one is a start (`root_admits`). With item 4 a start must also be a member of
  the domain label, or it is not launched (`root_admits_label` then; the anchor's row in `root_admits` now).
- **A single clause on Metal listed no roots for a Label domain** (`matcher_roots`): it listed them against empty
  labels. It lists the label's bound kinds now, which the program's first test, the seed, narrows.

## Measured

s1000 (339 files, 877,007 nodes). Metal runs are pinned (`RERAIL_LANE_PIN=gpu`). Every output relation's rows
were compared, before against after and native against Metal, after each step. They are the same.

The first run of a binary whose Metal source changed compiles the device's shaders: about 55 s of wall and
1,000 G instructions. That was 0390's and t45's "59 s anomaly". The figures below are from later runs.

Walk steps, internal_dead_code with t45's probe relations (`tmp/bench/pp`, load 20–50):

| Engine | Steps | Longest walk |
|---|---|---|
| before (d7) | 65.8 M | 63 |
| + items 2, 3 | 54.4 M | 71 |
| + item 4 | 51.5 M | 71 |
| + item 5, `all`s | 24.2 M | 85 |
| + item 5, lone walks | 19.8 M | 59 |

public_api_triples: 19.28 M → 7.43 M steps, the longest walk 78 → 59.

Captures, internal_dead_code on Metal (host CPU of `captured_rows`, by relation):

| Relation | Roots before | Roots after | CPU before | CPU after |
|---|---|---|---|---|
| `rr_ast_14_1` (`mn_function`) | 737,806 | 27,849 | 1,193 ms | 39 ms |
| `rr_ast_12_1` (`mn_dec_fn`) | 236,434 | 10,917 | 442 ms | 18 ms |
| all relations | | | about 1.95 s | 361 ms |

2,013,352 roots were dropped before any capture. Eight relations are restricted, all by `rr_ast_11`.

The whole run, in alternating pairs:

| Run | Before | After | Load |
|---|---|---|---|
| internal_dead_code native, items 2+3 (d7 → c2) | 14.0 s CPU, 178.8 G | 11.8 s, 134.9 G | 30–50 |
| internal_dead_code native, item 1 (c5 → c6) | 9.2 s CPU, 120 G, 2.5 s wall | 7.0 s, 92 G, 2.0 s | 25–28 |
| internal_dead_code Metal, item 1 (c5 → c6) | 8.2 s CPU, 96 G, 2.5 s wall | 6.2 s, 70 G, 1.9 s | 18–19 |
| public_api_triples Metal (d7 → c6) | 3.6 s CPU, 42.6 G, 1.2 s wall | 3.5 s, 42.9 G, 1.0 s | 18 |

The GPU's label kernels did not fall with the steps:

| Program | `label_micro_8` + `label_micro` before | after | `label_micro_8` dispatches |
|---|---|---|---|
| internal_dead_code | 80.9 ms | 66.7 ms | 60 → 78 |
| public_api_triples | 61.1 ms | 68.2 ms | 96 → 144 |

A seed is a label its readers wait for, so each seeded `all` adds a level, and each level is more dispatches.
And a pass still visits every 64-node group to read its mask, whether the group has a member or not.

## What this tells us

- The host was where the run's time went. Item 1 took 2.2 s of CPU from internal_dead_code natively (24% of the
  run) and 2.0 s on Metal, and items 2+3 took another 2.2 s natively. Both are generic: item 1 reads only the rules' joins, and item 2 only
  the pattern's shape.
- The device's steps fell 3.3× and 2.6×. Its time is now set by its dispatches and its scans of every group,
  not by the walks (Q187).

## Open

- Q187: the label kernels' time follows dispatches and group scans now. The next lever is launching a pass over
  only the groups its domain has members in, and merging the levels a seed adds.
