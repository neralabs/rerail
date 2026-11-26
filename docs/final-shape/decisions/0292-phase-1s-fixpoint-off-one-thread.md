# 0292. Phase 1's fixpoint off one thread: a walk that makes atoms walks in morsels, a load's chunks evaluate at once, and a node read goes with its document's join

Status: built (audited 2026-11-23). Decided (2026-11-15). Builds on 0278 §4 and 0279 §7 (morsels), 0254 (chunks at
once), 0109 (long-text twins), 0286 (a node's attribute asked within its document). Measured in notes/t40's follow-up.

## Intro

On all of PostHog (29,885 files) phase 1 answers the cone of requests: 29,771 loads, then the import fixpoint, one
recursive stratum of 50 relations (`py_anc`, `py_want`, `py_qlast`, …). A CPU timeline of the M3 Max showed 8–12
cores busy while the loads were answered, and then 1.6–1.8 s with 2 busy: the cone's thread in the stratum's joins,
and the store writer. The stratum's joins were walked whole (0.70 s): every big rule makes text on each row
(`$m = @pkg_up($m0, 1)`, `$q = @cat($m, $x)` split, `@head`, `@substr`, `@replace_all`), and 0279 §7 kept a join
that makes atoms off morsels, because in morsels its new atoms' ids would follow the threads' timing.

## Decision

### 1. A walk that makes atoms walks in morsels, deferring its new texts

A morsel interns nothing. A text it makes that the arena does not hold gets a deferred value of its own
(`setfix::Deferred`: `DEFERRED + i`, from 2^40 on: past every atom, which is 32 bits, below every node, whose
document is in the high bits), and the morsel's reads of a value (`name`, `held`, `known`, `@cat`'s parts) read the
deferred text. The same text in one morsel is one value, so its joins and dedup are as one walk's.

At the merge, every morsel's deferred texts are interned under one `Atoms::intern_all`, in the morsels' order. The
morsels are cuts of one walk's order, so this is the order one walk would have made them in: each text gets the
atom one walk gives it. Each morsel's rows then hold the atoms in place of the deferred values (in the head's
columns that may hold a made text, `made_columns`), and are settled, sorted whole, on the pool.

So no atom's id follows the threads' timing, and no morsel takes the arena's every shard to write a text.
`cut_tests::a_join_that_reads_texts_or_makes_atoms_walks_in_morsels_and_makes_one_walks_atoms` checks the rows and
the atoms against one walk's (each walk's connection has an arena of its own).

### 2. A load's chunks evaluate at once

The loads' chunks went through `drain` one at a time on the run's connection (0254 §1, "as before"); they now take
the parallel path, as the run's chunks do. The chunks' connections are closed once the loads are answered.
The answer-time report's split is renamed for what it measures: chunks "built and evaluated on workers", and
"begun and filed on this thread" (it never counted a chunk's evaluation, which was in the wait).

### 3. A node read goes with the join that binds its node

A long-text twin (0109) reads `t = @rr_node_text(v)` beside `rr_long_text_node(v)`. Both are a chunk's: only its
batch holds the node's text. Where the rest of the rule is cross-file, the split put the twin in the cross part,
which had no batch to read: `@rr_node_text without the run's long-text nodes bound`, or, on one thread, the last
chunk's batch, which holds no other document's nodes. `push_local_joins` now takes a node read
(`syntax::NODE_READS`) of a node its component binds into the component, exports its value (a `sym` column), and
pushes such a component whatever else would leave it in the cross part. A node's text is a function of its
document, as 0286 says of the requests.

`phases::tests::a_phase_hands_on_its_exports_and_its_reader_follows_each_edit` and
`oracle_tests::a_call_is_typed_by_the_overloads_its_positional_arguments_fit` failed so on many threads (the second
once the loads' chunks evaluate apart, §2); both pass.

## Measured

All of PostHog, the M3 Max, profiling build, K = 15, the same 29,747 findings in every run:

| | phase 1 | cone stratum | its joins |
|---|---|---|---|
| before (y1, y2) | 3.80 / 3.77 s | 2.90 / 2.91 s | 0.70 / 0.71 s on one thread |
| §1–§3 (x1, x2) | 3.60 / 3.53 s | 2.72 / 2.61 s | 0.30 s on its thread; 1.09 / 1.12 s CPU in 650 morsels |

§2 alone did not change phase 1 measurably (3.62 against 3.68 s): building the loaded documents was already the
pool's.

## Open

- The morsels' CPU is about 1.5 times one walk's (1.1 s against 0.7 s). Not yet attributed: the deferred table's
  string keys, the whole sort at the merge, or the threads' reads of the same relations.
- Rule walks of under 32,768 candidates still walk whole (`MORSEL_ROWS` × `MORSELS_PER_THREAD` × 2); most of this
  stratum's 16 waves are smaller.
