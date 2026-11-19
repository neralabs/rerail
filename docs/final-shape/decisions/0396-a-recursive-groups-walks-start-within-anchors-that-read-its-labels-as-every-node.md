# 0396. A recursive group's walks start within anchors that read its own labels as every node

Status: accepted (2026-11-18)

## Intro

A walk starts only within its anchor (0365): a node set, evaluated from the tape's columns and the finished labels,
holding every start the walk can emit from. The recursive groups' walks were left out (0365, "Not yet" 1). A
group's labels settle round after round, and an anchor reading them in the middle of the rounds could drop a start
that emits once they have grown. So every pass of a group started at every node its rule allows. On ten of
corpus0's files that was 5.4 million starts per round, for 0.3 million answers (Q133).

The user approved three variants, to be measured on the census before building one (2026-11-18, "go ahead"):

- (a) 0390's narrowing extended to the recursive utilities;
- (b) anchoring each group pass once, before the rounds, with the group's own labels read as every node;
- (c) both.

The plan also said to try semi-naive rounds only if the census showed the gap was worth it.

## Decision

1. **A recursive group's pass starts within its group anchor.** The anchor is the pass's `Set::anchor` with each
   label of the group read as every node (`Set::labels_unknown`, `anchor_bits`' `unknown`).
   - A set reads a label only as its members; a complement is a label of its own. So the anchor is a superset of
     the starts that can emit, whatever the group's labels hold: when they have settled, and in every round before.
   - It reads the earlier steps' labels, which are finished. So one evaluation, before the group's rounds, serves
     every round.
2. **On the CPU:**
   - A sweep (`sweep_steps`) evaluates each roots or targets pass's anchor once, before its entries. It enters no
     (node, pass) outside the anchor (`sweep.anchor_dropped` counts them).
   - Rounds (`label_level` in the group's loop) read anchors kept per program and tape (`Anchoring`, its `kept`),
     evaluated at the first round.
   - The levels past the groups keep 0365's anchors over the finished labels (`Anchoring` without a group).
3. **On Metal:** the plan gives a group pass's roots an anchor formula that reads the group's own labels as every
   node (`device_anchor`'s `unknown`). The runner evaluates a group's anchors (`Runner::anchors`, `label_formula`)
   before its sweeps, its fused dispatch (`label_group`) or its rounds. The walks read them through the root rule's
   anchored form (kind 2, `root_mask_in`, `root_admits`), as 0365's walks do.
4. **Not (a):** a recursive utility is read only under a move: its recursion moves forward (`matcher::lower`), and
   every move asks for every kind at the node it reaches (`walk_moved` and `count_moved` run under
   `demanding(None)`). So the kinds wanted where it is read are every kind, and 0390's narrowing has nothing to
   narrow. On the clauses: 1,365 reads of recursive utilities, none outside a move (`anchor_rows_per_answer`,
   `unmoved_recurs`). (c) is (b).
5. **Not semi-naive node-level rounds** (t45 item 4), on these numbers:
   - The best a per-round anchor can do is the anchor over the settled labels: 6.6% of the unanchored starts,
     against the group anchor's 20.9%. So it would save at most two thirds of the walks left.
   - That is about 0.6 s of CPU sweeps and about 135 ms of device time on ten files.
   - On Metal, the group anchors' one evaluation already cost about 98 ms of formula time. A per-round anchor would
     pay that every round.
   - d84724a6c's per-label sweep skip already passes over the nodes whose read labels did not change.
6. **The formula reader evaluates a set without remaking what it can read** (`compile_sets::Reader`, 2026-11-18).
   The group anchors cost about half of what they saved in the sweeps (0.9 s of 1.9 s). A census of their shapes
   (564 distinct, 6,940 evaluations) found three causes:
   - a label read as every node left `Or([…, All])`, and steps over every node (`Child { of: All }`,
     `Parent(All)`), each made by a pass over the whole tape;
   - long nested `or`s of kinds, each part a row of its own;
   - each label and text row copied before it was read.
   So now:
   - `labels_unknown` simplifies: an `or` with every node is every node (`Set::or`), nested ones flattened.
   - A step over every node is a row the tape's `Index` keeps (`over_all`), as it keeps the kinds' postings.
   - A conjunction tests first, then tests each child, descendant or following-sibling step only at the members
     left (`within`, as the device's conjunctions do).
   - An `or` gathers its parts into one row: a kind's postings set into it directly.
   - A label's or text slot's row is read where it is (`Row::Borrowed`), and a supertype's row where the index
     keeps it.
   The members are the same, and the 0036 formula passes, which use the same reader, get the same speedup.
7. **Locked by:**
   - `anchor_rows_per_answer` asserts on every tape that no start outside a group anchor emits with the settled
     labels.
   - `anchored_walks_find_the_unanchored_roots` and `on_metal_anchored_walks_find_the_unanchored_roots` find the
     same roots with anchors as without.
   - Under `RERAIL_PROF`, the CPU test also asserts that the groups' sweeps dropped starts.
   - `formula_rows_are_the_sets_members` checks the reader against the sets' members node by node (`set_members`):
     each label program's anchor, its group anchor and its formula on three tapes with their labels settled. 30,300
     sets; none differ.

## Evidence

All on the M3 Max, debug build, ten of corpus0's files, the machine under heavy load (load average 17 to 60). The
group anchors were toggled within one build by a temporary switch, removed before the commit.

- **The census** (`anchor_rows_per_answer`):
  - The groups' roots passes start at 5,408,169 nodes a round: 6,790 formula passes, which the groups walk, and
    100 micro walks.
  - Within the group anchors, 1,130,260 (20.9%). Within anchors over the settled labels, 356,447 (6.6%).
  - Answers: 309,136.
- **CPU** (`anchored_walks_find_the_unanchored_roots`, the wave's 31 waves, thread CPU, without against with the
  group anchors):

  | stage | without | with |
  |---|---|---|
  | `sweep.run` | 2,807 ms | 894 ms |
  | `sweep.walks` (with the anchors' evaluation) | 906 ms | 1,837 ms |
  | `anchor.eval` calls | 7,290 | 14,230 |
  | `sweep.entries` | 719 ms | 761 ms |
  | the sweeps in all | 4.43 s | 3.49 s (−21%) |

  The labelling's process CPU is dominated by the facts' rows (`fact_rows.tape`, about 20 s), and its totals moved
  with the load.
- **CPU, the reader of §6** (the same test, load average 45 to 50):

  | stage | before §6 | after |
  |---|---|---|
  | `anchor.eval` (all 14,230 anchors) | 1,361 ms | 542 ms |
  | the group anchors (against the 7,290 others' 443 ms) | about 918 ms | about 100 ms |
  | the sweeps in all | 3.49 s | 2.78 s (−37% against the 4.43 s without group anchors) |
  | `labels.formulas` (0036's passes) | 5.5–5.9 s | 4.9 s |
- **Metal** (`on_metal_anchored_walks_find_the_unanchored_roots`, `RERAIL_GPU_KERNEL_TIMES`, device µs, the warmed
  anchored run):

  | kernel | without | with |
  |---|---|---|
  | `label_group` (8, 32 and 64) | 599,620 | 200,702 (−67%) |
  | `label_formula` + `label_leaves` | 211,599 | 309,354 |
  | all device time | 844,730 | 543,861 (−36%) |

- **Tests:**
  - `a_misplanned_label_sweep_runs_again_and_finds_the_full_budgets_roots`;
  - `replan_finds_the_full_budgets_roots_on_the_pack` (CPU and Metal);
  - `lowered_clauses_find_the_same_roots_on_metal`;
  - `the_label_kernels_compile_for_every_format`;
  - `the_combined_graph_shares_labels_and_keeps_each_clauses_roots`;
  - `spilled_walks_find_the_cpus_roots` (12 files, every walk spilling).

## Not yet

- Passes with the same anchor are still evaluated once each, per tape (up to four passes a shape). After §6 the
  group anchors cost about 0.1 s of the sweeps' 2.8 s, so they are not deduplicated.
