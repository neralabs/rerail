# 0460. A pass starts within its demand: one formula read in its anchor, not a seed label

Status: accepted (2026-11-18). Numbered 0406 until the hub merge (on q178 too), 0460 on final-rerail-shape-rr-align until the sync branch's roll-up of it, and 0425 on the sync branch and benchmarks-on-integration until its merge into CUDA-support (2026-11-20). Supersedes 0459 item 5's seed labels. Follows Q288 (numbered Q187 where it was
written; the hub's Q187 is another item).

## Intro

A matcher's rule is lowered into label passes and a root program. Each pass starts at a set of nodes, its domain
(`Domain`). The question "where does a pass need to start?" has been answered five times, each time by its own
mechanism:

| Decision | Where a pass starts | Mechanism |
|---|---|---|
| 0248 | a walk past a rare check, from the check's rows | its own probe in the planner |
| 0365 | within its anchor, a superset of the starts that emit | a node-set formula (`Set::anchor`), evaluated on the CPU (`anchor_bits`) and on the device (`label_formula`) |
| 0390 | within the kinds where its reader can hold | a kinds list carried by the builder (`Demand::kinds`) |
| 0396 | a recursive group's passes, within anchors reading the group's labels as every node | the anchor formula again, `labels_unknown` |
| 0459 item 5 | inside an `all` that tests a neighbor, at a seed label of X's children or parents | a `Targets` label pass of its own, read as `Domain::Label` |

The user asked for Q187 (now Q288) and added: "we also seem to be re-solving this problem over and over, so please
survey the sibling worktrees and work on solving it more generically" (2026-11-18).

The survey: no sibling worktree has a decision on starts, anchors, domains, demand or seeds that this branch lacks.
The only hits were 0107 ("a search is demand rules"), the old number of this branch's 0144. Every one of the five
lives on this line.

Four of the five are node-set formulas, or could be. 0459's seed was not: it was a label made by a pass. A label is a
level its readers wait for, so each seeded `all` added a level and its dispatches (0459, "What this tells us").

## Decision

1. **Demand is a formula.** While lowering an `all`, the builder carries a `Demand { kinds, set }`. `set` is a node
   set (`compile_sets::Set`) that the node under test is in wherever the `all` can hold. For a conjunct `inside X`
   or `has X` by `neighbor`, through a field or not, it is the image of X's set by that one step:
   `Set::targets` of the same one-step program 0459's seed walked, from `Domain::set` of X's starts. An `all`
   inside an `all` conjoins its set with the outer one.
2. **A pass made there starts within it** (`Domain::Within(kinds, set)`). Its kinds are 0390's. Its set is read
   where the pass's anchor is read, ANDed into it:
   - on the CPU in `anchor_bits` (every caller: `evaluate_budgeted`, the fused walks, the sweeps);
   - on the device in `device_anchor`, by the roots each pass keys (`matcher_metal`'s `demands`).
   Where a pass has no anchor (not a micro walk, `RERAIL_ANCHORS=off`, a pattern pass), it starts at the kinds alone.
   That is a superset, correct for 0390's reason: narrowing only drops starts whose value no reader reads. The CPU's
   formula plans read the domain as one set (`Domain::set`), so a label they evaluate is the same.
3. **No seed label.** The neighbor conjunct is tested first in the `all`'s program. A conjunct is selective, so it
   sits in the program's own anchor, and the `all`'s roots label and the clause's domain need no other narrowing.
   A lone neighbor walk is a roots label like any other walk. It is a formula pass where its program is one.
4. **A demand's set has no text test** (`Set::texts_dropped`). A text slot is its own program's: in another pass's
   anchor, `Text(0)` is that pass's slot 0. Ruff F541's `has: {kind: string_start, regex: …}` beside a
   `not: {matches: has-placeholder}` gave 4 extra findings on Metal before this. Its test is the `fstring` clause in
   `seeded_and_copied_labels_find_ws_roots`. Dropping a test only widens the set.
5. **Ordering reads the set's labels** (`Domain::labels`: `Labels::reads`, the wave's borrowed labels, the sweep's
   reads). A pass waits for X's label where X walks, as the seed did, but not for a seed made from it. Renaming in a
   wave renames them (`Domain::relabelled`, `Set::relabelled`).
6. **`RERAIL_DEMAND_SETS=off`** is the measurement switch: an `all`'s labels start where the kinds allow alone, as
   before 0459 item 5. It replaces `RERAIL_SEEDS`.

What is left of where a pass starts is two things. Its domain says where its value is needed: the kinds, and a
formula. Its anchor says where it can emit. The two are ANDed, and one evaluator reads them on each side (decision
0310's "reads written once").

## Measured

The s1000 corpus: 339 files, 877,007 nodes. Engine `dm` is this decision. Engine `ms` is the hub merge plus
`cd3d2214d`, with 0459's seeds. Metal runs are pinned (`RERAIL_LANE_PIN=gpu`), with t45's probe relations
(`tmp/bench/pp`). Load was 13–21, alternating runs. The GPU kernels' times are from the command buffers' timestamps.
The runs were under the default memory line (`RUN_MB`), which cut s1000 into 14–17 chunks; the times are those
chunks' (0419).

| Program | Engine | `label_formula` (dispatches) | `label_leaves` | `label_micro_8` | formula groups | formula images |
|---|---|---|---|---|---|---|
| internal_dead_code | ms | 93.1 ms (159), 128.1 ms (150) | 30.4, 40.8 ms | 8.4, 9.0 ms | 28,622 | 624, 584 |
| internal_dead_code | dm | 117.3 ms (140), 118.4 ms (124) | 37.1, 44.0 ms | 9.5, 8.4 ms | 22,520, 20,786 | 320, 280 |
| public_api_triples | ms | 164.0 ms (184), 121.2 ms (124) | 43.6, 36.8 ms | 13.2, 8.3 ms | 17,616, 16,874 | 376, 244 |
| public_api_triples | dm | 133.4 ms (148), 97.5 ms (108) | 40.3, 30.0 ms | 12.7, 9.9 ms | 12,109 | 136, 96 |

- **Rows:** the same for every output relation: `dm` against `ms`, natively and on Metal, and natively against
  Metal.
- **Formula groups** (the (file, formula) pairs `label_formula` evaluates) fell 27% and 31%. **Formula images**
  (device formulas sent) fell about half and two thirds: each seed was a formula pass of its own.
- **`label_formula`'s time did not fall.** The final binary (with item 4), in alternating pairs at load 74–91:
  internal_dead_code 95.5–98.1 ms → 90.6–94.7 ms (159 → 116–140 dispatches), public_api_triples 94.5–99.2 ms →
  94.2–97.5 ms (164–194 → 148). The groups and images fell the same: 22,520 and 12,109 groups, 260–320 and 136
  images. The table above (the binary before item 4, load 13–21) is within the same noise.
- **Whole runs:** about the same (final binary: internal_dead_code 7.0–7.2 → 7.3–8.0 s of CPU, 56.1–56.7 →
  56.2–57.1 G instructions; public_api_triples 6.3–6.5 → 6.2–6.3 s, 48.3–49.1 → 47.0–49.1 G). internal_dead_code on Metal took 7.1–7.2 s of CPU before and 7.5–7.6 s after;
  public_api_triples took 5.9–6.2 s before and 5.6–6.5 s after. Natively, public_api_triples took 4.8–5.3 s of CPU
  before and 4.8–5.5 s after (55.3 G instructions before, 56.3–56.9 G after).

At load 50–90 earlier the same pairs gave internal_dead_code `label_formula` 150–167 → 79–110 ms. Those are noisier,
and not used for the verdict.

## What this tells us

- Q288's premise is gone. 0459 saw `label_micro_8` + `label_micro` at 66.7 ms and their dispatches rising with the
  seeds. On this line (with 0365's anchors from the hub) the micro walks are 8–13 ms, and `label_micro`, the loop
  tier, no longer runs. Launching a pass over only the groups its domain has members in would save at most those
  8–13 ms. It is not built.
- The work removed was fixed per seed pass, not per pair: fewer pairs and images did not make `label_formula`
  shorter. What this change buys is shape: one mechanism for where a pass starts, no level for a seed, and 1,595
  fewer targets labels in the pack's wave (1,886 → 291, `wave.rir`).
- The label passes' GPU time is now `label_formula`, 90–130 ms: the anchors' and formula passes' evaluation, about
  5 µs per (file, formula) pair. Each pair is word-wide set operations over the file's nodes. The kernel's time
  follows how many formulas there are and how large they are, not dispatches. Q288 now names that.

## Open

- Q288: `label_formula` evaluates each (file, formula) pair apart. Formulas of one level share their tests (the
  same kinds, the same neighbor sets), and each pair evaluates them again.
- Q289: a level's fact constraints (the fact rows' `(node, label)` pairs, made on the host) were uploaded once per
  pattern kernel group, the same buffer each time. 32 MB (8.1 M words) went up on 15 fixtures. They are uploaded
  once a level now (`Runner::level`). That one upload is still a copy of the facts' rows (AGENTS.md, 0353): the
  device should read the fact's relation in its block.
