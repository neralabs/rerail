# 0229: Past the step that binds the head, a rule's walk needs one witness

Status: built (2026-11-14). Refines the native set engine's walk (`setfix`) and the planner's pricing of an order.

## Intro

A rule's walk binds its variables one step at a time and emits the head's row at the end. Some rules bind every
variable the head reads early, and the rest of the body only checks that a match exists. Each match the rest finds
makes the same head row again.

The split of an export into its parts (decision 0097) makes such rules: each part's key.

```
fzk281_fl273_violation($d) :- demand_fzk281_fl273_violation($d), node($d, $v, _, _, _), line($v, $l), col($v, $c).
```

`$d` is a document and `node($d, $v, …)` is every node in it. The head needs `$d` alone, yet the walk read every node
of each document. A finding's location (`rr/lower/findings.rs`) has such keys for its line, and since the hub added
the column, for its column too. On PostHog these keys cost about 60 s of CPU in a cold run: 1,000× to 27,000×
fan-outs at `node`, 10.6 M rows for 7,086 keys at one step.

The engine already stops at one row of an atom that binds nothing (a semi-join). It did not stop for a run of atoms
that bind only what the head never reads.

## Decision

**The walk stops at one witness past the head** (`setfix::tail`). A join's tail starts after the last step that
binds a slot the head's terms read. In the tail, once a row is made, each step's loop ends: every other row there
makes the same head row. A walk that collects rows (a closure's rest, a level's pairs) keeps every row.

```
let made = cx.n;
go(cx, i + 1, slots);
i >= cx.cut_from && cx.n > made        // the head's row is made: this step's loop ends
```

**The planner prices that cut** (`plan_search`'s `step`). In an order, a step after the head's variables are all
bound passes on no more rows than it receives, and reads no further rows than it passes on. Otherwise every order
looked alike, since each walked all of a document's nodes. The planner then started from `col` (est. 31,191 rows,
4 M read), where starting from `demand` reads one node of each document.

## Consequences

- PostHog, profiled cold run:
  - the native joins' probes take 82 s of CPU, down from 139 s;
  - the joins inside each file take 92 s, down from 149 s;
  - instructions fall to 3.41 T from 4.29 T;
  - the location keys no longer appear among the joins' heads;
  - the 27,884 finding locations are the same.
- The step counts for such a rule are the rows walked, not every witness. Replanning reads them as the rule's real
  fan-out.
- The device's walk does not cut yet. On Metal such a key still walks every witness. (Decision 0277 adds the
  device's witness stage.)
- Fixed along the way (decision 0170): an atom segment was cut from every atom held when it was written, not only
  up to the write's end. When another thread interned atoms during a write, the segment ran past that end. The write
  recorded its end as the planned one, and the next write took the store's end for another process's. So a cold run
  in a private store forked (PostHog: 1,676,444..1,680,880). Segments now end at the write's end
  (`Interner::segment`).
- Tests:
  - `atoms::tests::pending_atoms_are_written_in_segments_of_bounded_size` (a segment never past its write's end);
  - the logic crate's `native::` and `corpus::` tests;
  - the engine's `plan::tests` (60). `dot_of_the_empty_name_is_the_name` had failed before this change: its two runs
    shared a store, so the second found the first's strata kept and the native set engine solved nothing. Each run
    now has a store of its own (`run_both`).

## 2026-11-21: the cut is priced over the order, not a step at a time

Pricing `step`'s cut one step at a time assumed each walk's first row was a witness. Where no row passes the rest of
the body, the walk reads all of a binding's rows, and the cut priced that walk at one read a binding. The search
already dropped that cut (Q411). It counts every row walked and scales the tail once, by its witnesses
(`witnessed`). The tail is read in full where fewer rows come out than bindings went in. The greedy order (decision
0123 §7) and the explain record's step rows still took the per-step cut.

Decision 0431 R9's rules meet that case. One is the key over a tape value's findings,
`exists_N($d) :- demand($d), node($d, $v, …), !placed($v), finding($v)`. The other is a placement through a guest's
string, factorized as `fzk…($d) :- demand($d), node($d, $s, …), guest_place($v, $s, $l, _), …`. Both were led from
40 documents, with `node` by `$d` priced at one row a document: 118 rows of work. That is under a search's cost, so
the greedy order was taken without a search. Each call read every node of its documents, 100 k rows, against about
80 when the finding's one row is read first. On corpus0, one pack, 343 such strata took 19.9 s.

- **The change:** `step` is now `step_all`. The greedy order keeps its tail as `price` does, and is `witnessed` at
  the end. A fixed order, the search and the greedy order now price a step the same way.
- **Corpus0 × B-now, off, one thread:**
  - one pack: 1.591 T → 1.314 T instructions;
  - normal packs: 3.838 T → 3.157 T;
  - the same 463 findings (`d067ec2ebf6f`) in both.
- **Test:** `plan::tests::a_walk_past_a_bound_head_reads_every_row_where_none_is_a_witness`.
