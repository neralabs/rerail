# 0256: The walks, the heavy-key split and the trials read the rule's body

Status: built (2026-11-15). Closes decision 0248's last open item. `WalkSnap`, `Walker`, `heavy_link`,
`Run::heavy_parts`, `Run::split_spike` and the driver's trials in `plan.rs`.

## Intro

Since the merge of Typing-support's handles (t36 item 7), the planner prices orders from a rule's `Body`: a view of
the version's tokens, with variables as slots. A few parts were left on the old form. These were the background
walks, the split of a walk's lead rows by a heavy key, and the trials at a cut. Each time, they decoded the rule
into a tree (`Run::tree`), once per cut or search request.

## What changed

1. **The walks read the body.** `WalkSnap::take`, `checks`, `link`, `starts` and `walk` take a `&Body`.
   - `Walker` holds the body, and keeps its bindings by `VarSlot` index, not by name.
2. **One heavy link.** `heavy_link(b, lead, has)` finds the column a walk's lead shares with a heavy key. It is the
   one definition behind `WalkSnap::link` and `Run::heavy_parts`; before, each had its own copy.
3. **The trials read the body.** The cut's trial code uses `b.view`, `b.args`, `b.rel` and `positive_atoms`. It
   no longer decodes the rule.
4. **A check's wildcard is no column of its probe.** When a walk tested whether a lead row passes a check such as
   `k(n, _)`, it keyed the wildcard's column with a value. That value was the lead's value of the first variable
   it found, the wrong variable.
   - So no row passed, and the walk priced the order as if the check removed every row.
   - Now a wildcard leaves its column out of the probe.

`Run::tree` remains only where the planner makes new rules from a rule's parts: lattice and linear specs (Typing-
support's step 5), and the plans' text dump.

## Measured

The findings are unchanged: 545 on the fixture, and 3,462 on the first 50 data-layer files, warm and from scratch.

## Tests

- `a_wildcard_in_a_check_is_no_column_of_its_probe` (`plan/joins/tests.rs`): `h(f, m) :- r(f, n), k(n, _), r(f, m)`
  over 100 functions of 10 nodes, half of them with a kind. The walk reads 1,500 rows of work.
- The walk tests take the rule's body (`body::alone`, `body::nth`).
- Planner 133 (1 ignored), `native::` 52 (2 ignored), `cut_tests` 4.
