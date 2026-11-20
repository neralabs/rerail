# 0511. An aggregate bounded by its probe is an as-of search, not a helper per probe

Status: proposed (2026-11-20). t50 step 3; follows 0476.

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

## Not decided here

- Steps 2 (group argmin) and 5 (keyed nearest) are operators of their own. Group argmin may reuse the bound's sorted
  read.
