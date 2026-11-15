# 0143: The planner picks how each kind-set axis is made: per looked-up node, or for the batch by the transducer

Status: proposed (2026-11-10)

Numbered 0105 where it was written; renumbered 0143 in the hub (2026-11-13) because another record had 0105.

## Intro

A rule asks tree questions through three axes:

- `nearest(n, a, set)`: `a` is the closest ancestor of `n` whose kind is in the set;
- `inside`: every such ancestor;
- `has`: `n` is in the set, and `a` is any of its ancestors.

Decision 0036 had the transducer's file-local FIX make these relations. The FIX is one pass in preorder on the CPU,
or pointer jumping on Metal. Decision 0057 changed that: each relation is made per node a join looks up. Building
every set for every node, before any join ran, cost 2.6 s on the 9,010-file corpus, and most joins look up only a
few nodes.

Some sets are looked up a lot, though. In the Python pack, `rr_nearest_ks12` takes about 300,000 lookups over
873,000 nodes. Each lookup climbs the tree, stores its rows and records them under a lock. For a set read that
often, one solve of the whole batch is cheaper.

This record lets the planner choose per set. It already prices every plan step by step, so it knows how many keys
reach each axis atom.

## Decision

### 1. The planner says how many keys it expects

After the native run plans a seed or a delta join, it walks the plan's steps (`plan::expect_axis_lookups`). Each
step that reads an axis by its `n` column passes on the rows the planner expects to come in: the rows after the
step before it.

```rust
// A source made on lookup can take the hint; every other source ignores it.
fn expect_lookups(&self, column: usize, keys: f64) {}
```

The step-by-step plan is now also recorded when the body reads an axis, not only for the profile and the device.
Wrappers (`retain::Renamed`, the overlay) pass the hint to the source they wrap.

### 2. The axis chooses once the lookups cost more than a solve

`within::Within` adds up the expected keys of `nearest` and `inside`:

```text
choose the solve when  expected keys × LOOKUP_NS  >  batch nodes × SOLVE_NS
                       (150 ns a key)                (12 ns a node)
```

The two prices are estimates, not measurements (see Open). The choice is made once. From then on, `n` lookups
read the solve.

`has` reads every ancestor of a member, which the FIX doesn't compute, so `has` stays on demand.

### 3. The solve is the transducer's FIX

`typing::file_local::nearest` runs over the batch's parents, with membership as the kind (the set is `{1}`). It
runs on the run's binding: pointer jumping on Metal, or the CPU's preorder pass. From the answers:

- `nearest`: each node's row is its answer;
- `inside`: each node's rows are the chain of answers, the answer's answer and so on.

They are kept as one run of rows per node (`Solved { start, pairs }`).

Row ids stay stable, and the two ways of making rows coexist:

| ids | rows |
|---|---|
| `0..` | the solve's rows |
| from `1 << 30` | `n` rows made per lookup before the solve was chosen |
| from `1 << 31` | `a` rows, always made per lookup |

A scan of `nearest` or `inside` (reading every row) runs the solve, since it needs every node anyway.

## Why

- **The cost depends on the set.** 0057 measured the eager form over every set. Most sets get a few lookups, and
  for those 0057 holds. A few sets get lookups in the hundreds of thousands, and for those the per-node path's
  locking and storing is the larger cost. The planner already has the number that separates the two cases.
- **Decision 0036's form comes back where it pays.** The axes are again made by the transducer, on the GPU when the
  run is bound to Metal, but only for the sets that need it.

## Evidence

- `facts::vtab::syntax::tests::the_kind_set_axes_hold_exactly_their_pairs` checks each axis against a brute-force
  walk of every node's ancestors, three ways: made per lookup, solved on the CPU, and solved on Metal. It covers
  lookups by either column, the scan, and the row count from the tree.
- `within_holds_exactly_the_contained_pairs`, `plan::tests` and `typing::file_local` are unchanged.
- The Python pack over the ruff fixtures (486 files, `--metal`): violations and policy rows are identical.
  - Solved sets: 54 (set, chunk) choices.
  - Metal: about 100 launches, 1.6 ms of GPU time, 134 ms of host wait in all. The wait blocks no CPU.
  - CPU (user + system), interleaved with the merge binary at matching load: 38.2 → 37.5 s at load 123–134, and
    31.2 → 30.3 s at load 56–61. A third pair is left out because the load fell between its two runs.

## Prior work consulted

- **0057 §2:** the per-node axes, and the measurement that eager sets for every node cost 2.6 s. This record keeps
  them as the default.
- **0036:** the axes as transducer built-ins, made by the file-local FIX.
- **0038/0040:** pointer jumping on Metal, one dispatch per piece.

## Departs from

- **0057 §2's "`with_nearest` no longer calls the file-local FIX".** The FIX runs again, for a set the planner
  expects to look up more than a solve costs.

## Open

- **Measure the two prices.** `LOOKUP_NS` and `SOLVE_NS` are estimates. They should come from the cost statistics
  (decision 0100 §14), like the accelerator's other prices.
- **Place the solve.** For pieces of about 30,000 cells, a Metal launch waits about 1 ms, while the CPU pass takes
  tens of microseconds. The executor's pricer should choose the device, not the run's binding.
- **Estimates can be far off.** In the profile, plans estimate 0.25 keys where 55 arrive. The choice is only as
  good as those estimates.
