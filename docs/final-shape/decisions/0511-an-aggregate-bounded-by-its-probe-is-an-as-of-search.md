# 0511. An aggregate bounded by its probe is an as-of search, not a helper per probe

Status: accepted (2026-11-20), steps 1, 2, 4 (SQL) and 5 built; the search of step 3 not yet (§ Built). t50 step 3; follows 0476.

## Intro

The rules for "the binding a read sees" ask for the latest of a group's values before a bound the read gives:

```
rf_fn_seen_at(i, u) :- rf_fn_read(i, s, x, r), !rf_crossing(i, s), u = max t : { rf_fn_at(s, x, _, t), t < r }.
rf_seen_at(h, u) :- …, look(h, x, s), rf_start(h, r), u = max t : { binding(s, x, b, _), rf_start(b, t), t < r }.
```

W does this with a binary search in the scope's bindings of the name (`ScopeTable::visible`). The engine does it like
this:

- The aggregate's body reads `r`, a variable bound outside it. So the lowering makes a context helper
  (`rr_ctx48(r, s, x)`, the probes) and an aggregate helper that carries `r`:

  ```
  rr_agg47(r, s, t, x) :- rr_ctx48(r, s, x), rf_fn_at(s, x, _, t), t < r.
  ```

- That is a row for each probe and each value of its group before it, and the `max` then keeps one per probe. t50
  measured `rf_seen_at` at 2.2 M rows made to keep 17 K on `all`. T5, the binding a read sees, is 7.7 s of the rules'
  CPU there, the largest item steps 2–5 address.

The work is proportional to the probes times their groups. It should be proportional to the probes, with a log factor
for the search.

## Decision

1. **An aggregate is bounded by its probe** when:
   - its op is `min` or `max`;
   - its body is atoms plus exactly one comparison of the aggregated variable with a variable bound outside it
     (`t < r`, `t <= r`; for `min`, `t > r`, `t >= r`);
   - no other literal of the body reads an outside variable that is not a key of the group.

   The front keeps its syntax. Nothing changes in `.rr`.
2. **The lowering keeps the bound on the aggregate.** The body without the comparison is the group's timeline: one
   helper over the group's keys and the value, `rr_asof(s, x, t) :- rf_fn_at(s, x, _, t)`, which does not carry the
   probe. The aggregate literal reads that helper and carries the bound: `u = max t : { rr_asof(s, x, t) } below r`.
   The bound is the aggregate's op, as `join`'s keys and separator are: `AggOp::Bounded { max, bound, strict }` (and
   the image's `AggRef::Bounded`). `Literal::Agg` keeps its fields, so the literal's 45 exhaustive matches stay as they
   are, and the compiler names each place that reads an op. A body of one atom with only wildcards besides its keys
   and the value needs no helper.
3. **The native engine searches for it.** A scalar step with a bound reads its source through an order on its key
   columns then the value. That is the group's run, sorted by the value, and a binary search finds the last value
   under the bound (the first over it, for `min`). With no such order the step folds the group with the bound as a
   filter. That is still one pass over the group per probe, and nothing is materialized.
4. **Every other backend states the bound or refuses the stratum.**
   - SQL: `(SELECT max(t) FROM rr_asof WHERE <keys> AND t < r)`.
   - The generated tiers: until they read it, a stratum holding a bounded aggregate is placed native.
   - The device: it refuses the stratum, naming why. Its form, a merge path over the
     sorted probes and the sorted timeline, is a later step. Metal work waits on the 2026-11-20 crash.
5. **Tests first.**
   - The front lowers each recognized shape to a bounded aggregate, and leaves an unrecognized one (a second outside
     variable in the body) as it was.
   - The native step's value equals the fold's on a group of 10⁴ values and 10⁴ probes, ties and empty groups
     included. Its work is proportional to the probes, not their product.
   - The stage harness's rows equal W's on its cases, the projects and s1000, and the findings replay is unchanged.

## Built

- **Lowering** (`rr::lower`, `Lowerer::bounded`): the shape of §1, for a value a declared `int` column holds. Nodes and
  symbols are left as they were, since they order by keys `front::order_nodes` writes later. `RERAIL_ASOF=off` turns
  the recognition off, for measuring.
- **The op** is `AggOp::Bounded { max, bound, strict }` (the image's `AggRef::Bounded`, code 6: the bound's variable,
  then its flags). The bound is a variable the literal reads outside its atom. Patterns written with `..` do not name
  it, so the compiler cannot find the places that list a literal's variables. They were found by hand and each now
  lists the bound: the variable sets (`Literal::vars`, the plan's, the version passes'), readiness (`ready`, magic),
  the renames, the use counts, and the digests and canonical keys.
- **Native**: the spec's `maxlt|maxle|mingt|minge` items and `Step::Scalar`'s `bound`. The step folds the group's
  candidates with the bound as a filter, so nothing is made per probe. It shares no summary bag (`scalars::plan`), and
  liveness counts the bound as read.
- **SQL**: the aggregate's subquery gains `value op bound`.
- **The device**: §4's refusal was not built. The device's compile of a scalar step matched it with `..` and
  folded a bounded step as a plain min or max. The coverage gate's device column showed the wrong rows (0515, found on
  a run that reached Metal by mistake). The compile now refuses a bounded step (`setfix::device`), not yet run on a
  device. The four probes (`aggregate max bounded by <`, …) are native and equal SQLite's rows. Their device column
  needs writing on a Metal host once Metal work resumes (`RERAIL_COVERAGE_NO_DEVICE=1` runs the native side alone).
- **Tests**: `setfix::bounded_tests`. The lowering test covers each side, either way round, and three shapes left
  alone. The step test checks 10⁴ values and 10⁴ probes against the plain search, on all four sides.

## Measured (same host, 2026-11-20; every stage's rows unchanged, on or off)

| Strata (`RERAIL_STAGE_TOP`) | `all`, off s | `all`, on s | s1000, before s | s1000, after s |
|---|---|---|---|---|
| `rf_fn_seen_at` | 1.574 | 0.731 | 0.170 | 0.069 |
| `rf_seen_at` + its demand | 0.859 | 0.829 | 0.251 | 0.224 |
| `rf_mlatest`, `rf_mlatest_at` | 1.012 | 1.016 | — | — |
| the per-probe helpers (`rr_ctx48`, `rr_agg47`, `rr_ctx52`, `rr_agg51`) | 0.927 | — | 0.074 | — |
| the group helpers (`rr_agg46`/`44`, `rr_agg47` and its demand) | 0.303 | 0.491 | — | — |
| **the as-of strata** | **4.68** | **3.07** | | |

On `all` the as-of strata go from 4.68 s to 3.07 s of CPU. The groups are small (a name's bindings in one scope), so
a probe's fold reads a few rows, and the binary search of §3 is not worth building yet. What is left in
`rf_seen_at` and `rf_mlatest` is their own joins, not the aggregate. `rf_mlatest` keeps its unbounded max, which needs
no probe.

## Not decided here

- Steps 2 (group argmin) and 5 (keyed nearest) are operators of their own. Group argmin may reuse the bound's sorted
  read.
