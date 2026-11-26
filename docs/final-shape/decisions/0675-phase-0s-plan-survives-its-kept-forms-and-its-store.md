# 0675. Phase 0's plan survives its kept forms, the CPU walk, and a new process

Status: done (2026-11-24).

## Intro

A fresh policy on the merged tree failed at its first read: `core.rerail: in no module of phase 0's plan`. The
benchmark worked around it with a `.rerail` copy of its `.rr` rule file (the ".rr includes" finding). Phase 0
(decision 0639) plans a program's library as modules. Each module is named `variant:digest+digest`, made with
`join d by d sep "+"` (`p0_module_of`), and has a key, a rank and a scope. A graph is kept three ways:

- in a process map;
- as a shipped image;
- as kept facts in the store (`Images::facts`, `graph_of`).

The plan was lost in three places. Each one is fixed at its cause.

## 1. The kept facts held no plan

`Images::facts` wrote a graph's includes, digests and phases, but not its module plan (`g.plan`) or its modules'
images (`g.lowered`). A graph read back from the store had no plan, so a text composed of its modules resolved none.

Fix:

- The facts now hold `p0g_module(name, rank, key, scope)` and `p0g_lowered(key, image)`.
- `graph_of` reads them back.
- The facts' key is bumped (`graph facts 2`), so facts kept without a plan are not read as a graph.

Test: `a_graph_kept_and_read_back_keeps_its_plan` takes `core.rerail`'s graph through both kept forms (facts and
image). It checks that each form is the same graph, plan included.

## 2. A test of a bound slot dropped texts its own walk made (CPU)

The merge brought deferred texts (decision 0666 and the decisions near it):

- A text a walk makes is interned at `settle_deferred`, after the walk.
- Until then, a probe of a relation treats it as no atom (`before_walk`): no row of a relation can hold it.

`m = cat(w, ":", j)` lowers to a `str_concat` that makes `m`, then `str_between` tests with `bind: false`. The tests
recover `w` and `j` from `m` and check them against the slots already bound. Those tests are not probes of a
relation. Their value is compared with a slot. Where `j` was a `join` the same walk made, `before_walk` turned it
into "no atom", and the row was dropped. So every module of two or more files (`core:a+b`) was planned by no row.

Fix: `text_value` and the plain `Cat` read a step with `bind: false` as the slot's value, `held(t)`, without
`before_walk`. A step that binds a slot still reads a probe's value, as before.

The device was right: its atoms find (`ROW_FIND`, decision 0666) is set only where a step binds a slot. With
`bind: false` it compares texts (`TEXTOP`).

Test: `a_cat_of_a_text_its_join_made_keeps_its_row`. It has three groups whose members carry a nonce, joined by `"+"`,
and then `m = cat("core", ":", j)`. Before the fix it returned only the one group with a single member.

## 3. A new process lowered every module its store already held

Commit aec6f2539e made phase 0's answer to `p0_lower_req` read no store inside the waves, as decision 0104 requires.
Before that commit, the answer asked the image store by alias, which is SQL. After it, the answer took only modules
the process held. A process that had not made a module itself lowered it again, even though its store kept the
module's image. That covers a test whose store is copied from the template, and a project's next process.
`phase_0_lowers_each_module_its_store_does_not_hold` was red under nextest, where each test is its own process.

Fix: both invariants are kept.

- Before phase 0's first wave, `derive` calls `rr::images_ahead`. It makes one read
  (`ContentStore::pinned_sources_by_alias`): every image the store pins, under each of its aliases, with where it
  lies in its segment and its lease.
- Inside the waves, `held_or_ahead` looks a module's key up there and reads its image in place
  (`ContentStore::source_at`, a mapped segment, no statement). It decodes the image and holds it for the process.
  It checks the shipped bundle first, as `read_image` does.
- A lease that is running down is renewed behind the run (`renew_image_leased`, queued). The lease was read in the
  same read before the first wave.
- An image whose segment was compacted since that read is not found. Its module is lowered, as one never kept would
  be.
- A module's scope is looked up the same way, so a new module over modules the store holds is lowered once
  (`before + 1` in the test).

Tests:

- `phase_0_lowers_each_module_its_store_does_not_hold`;
- `r5_the_runs_read_no_sql_inside_the_waves` (`RERAIL_SQL_STRICT`), still green.

## Also

`sum_and_join_lower` (tests-rr) expected an aggregate's helper column to be named after its variable. Since roll-up 99
(rr-align), a helper named by its content has its columns named by place (`rr_c<i>`), so two lowerings declare it
alike (decision 0639 §3). The test now checks the value by its argument's place: an `int` column.
