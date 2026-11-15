# 0233: The device reads axes whole, holds only the columns it reads, and keeps one witness past the head

Status: built (2026-11-14). Refines the device's joins (`setfix::device`, `metal_join`, decision 0095), placement's
prices (`plan::placement`) and the tree axes' batch solve (`within`, decision 0143).

## Intro

On Metal, PostHog's cold run took 582 s against 327 s on the CPU, for about the same CPU time. A lighter run shows
the same four problems in seconds: one rule (`builtins.len` calls) over PostHog's `posthog/hogql`, 437 files
(`tmp/light/run.sh`, its data in this tree).

1. **Strata refused.** A stratum that reads a tree axis by keys was placed on the GPU, then refused:
   "an atom read by keys over a relation made on lookup". The axes (`rr_nearest_<set>`, `within`) make their rows
   per node looked up. The device reads a relation whole, so it would make every row. On PostHog `wstart`, `walk`
   and `pre_115` cost 17 s of CPU after their refusals; on hogql, 1.1 s.
2. **Uploads.** Each stratum's copy of an input went to the device again: 340 uploads of 506 MB on hogql, 42 found
   resident. A copy was keyed by the engine's relation, and each stratum's view of `node` is a relation of its own.
   Every column was uploaded, read or not, and a text test's relation was uploaded only to hang its verdicts on.
   On PostHog the device's budget ran short: 2.2 M rows of `sym_of` were refused for capacity.
3. **Host cost unpriced.** `kind(n, k) :- node(_, n, k, _, _)` copies `node`. On the GPU it cost what it costs on the
   CPU (on PostHog, 13.8 s against about 5 s), because every row comes back and is stored either way. Placement
   priced neither the upload nor the store's insert of each row back, and priced a scanned row like a probe.
4. **No witness on the device.** Past the step that binds the head, the CPU's walk keeps one row (decision 0229).
   The device's stages are breadth-first: they made every row, `node` × each document's nodes for one row a
   document.

## Decision

### 1. An axis read whole is solved for the batch

A source made on lookup says whether it can make its rows at once (`Source::complete_rows`). The kind-set axes
can: the batch's solve (decision 0143) is one pass over the nodes, by pointer jumping on Metal. Containment in the
tree (`within`) now has the same solve, as `inside` of every kind: each node's ancestors. The device's keyed read
of such a source chooses the solve, and the CPU's lookups then read the same rows. A span axis has no solve and is
still refused, named. On the CPU, containment is never solved by the planner's expected lookups (its solve is every
ancestor of every node); only a reader of the whole relation chooses it. A scan of containment reads the solve too.

### 2. One upload per source, of the columns read

A device copy belongs to an owner: the source a view reads whole (every view of it, any columns), or the relation
itself (a derived relation, or a view with constant filters). It holds only the columns the stages read: keys,
column tests, binds, the row program's columns, an aggregate's column (`right_cols`). A stage reads them where the
copy holds them (`narrowed`). A stage that needs a column the copy lacks uploads it again with the union, and keeps
the copy's texts and verdicts. A text test's or text function's entry uploads no column. A source's rows are read a
row at a time (`Source::row_at`), not a value at a time.

### 3. Placement prices what the device's host does

- Each relation the stratum's atoms read is uploaded: its rows × the columns an atom reads, at 7 ns a value.
- A head row back costs 75 ns: its read into the store and the store's insert after the wave.
- A row a CPU scan reads costs 30 ns, not the 100 ns of a probe's unit of work.
- A step past the bound head is priced on the device with every row it would make (`StepPlan::all`), not the
  CPU's one witness, where the device cannot witness it (§4).

The values were measured on the hogql run (2026-11-14); the comments at the constants give the measures.

### 4. The device keeps one witness

Where every slot the head reads is bound before step `i`, and the rest is an atom over an input relation that binds
slots, then at most three atoms (or negated atoms) over input relations keyed by slots bound before, constants or
the first atom's columns, with no fused program, the rest is one stage (`mj::WITNESS`). Per binding it walks the
first atom's candidates in order until one that every probe passes, and the binding goes out as it is (a semi-join's
scatter). Each loop is bounded by its candidate range. Any other shape keeps the breadth-first stages.

## Consequences

- hogql, profiled cold run on Metal (load average 60–80, so wall is noise):
  - no stratum is refused for an axis; `wstart` runs on the GPU in 54 ms of CPU against 0.50 s on the CPU;
  - uploads fall from 506 MB in 340 uploads (42 resident) to 56 MB in 88 (103 resident), their host CPU from
    411 ms to 31 ms; the GPU strata's host CPU from 659 ms to 52 ms;
  - `kind` stays on the CPU;
  - the process takes 6.9 s of CPU against 7.1 s on the CPU binding; 1,629 violations either way.
- PostHog, profiled cold runs of the same build at the same load (average 63–66), 29,885 files: the outputs are
  identical (29,747 violations).
  - On the CPU: 487 s wall, 446 s CPU, 3.51 T instructions.
  - On Metal: 654 s wall, 481 s CPU, 3.66 T instructions.
  - On Metal, no stratum is refused for an axis, and 2.2 M rows of `sym_of` are still refused for the device's
    budget. Uploads total 906 MB, against 14,435 copies found resident.
  - `wstart` runs on the GPU 2135 times, about 14 files a run. Its price says 3.56 s on the device against 7.36 s
    on the CPU. In fact its device runs take about 7.5 s of host CPU, the same as on the CPU (7.69 s), plus 17.8 s
    of GPU time. The price leaves out the pass after the wave: 1.2 s, about 236 ns a row back on PostHog's store,
    against the 75 ns measured on hogql.
  - The extra wall time is spread over every stratum the main thread waits on, including those on the CPU (`kind`,
    `literal_str`), with the same CPU in each. The waits on the GPU's AST matcher batches double (9.5 → 19.9 s),
    which accounts for about 10 s of it.
- Open:
  - Placement still prices every upload, though a relation an earlier stratum of the chunk uploaded is found
    resident.
  - A device subgraph's results still come back to the store between strata.
  - The device price omits the after-wave pass and the GPU lane's latency.
- Tests:
  - `setfix::device::tests::past_the_bound_head_the_device_keeps_one_witness` (the CPU's rows; a binding per
    document, never per node);
  - `vtab::syntax::tests::within_holds_exactly_the_contained_pairs` (the tree read whole by its solve, before any
    lookup);
  - `placement::tests::a_stratum_is_placed_by_its_price_and_named_by_the_forms_the_device_lacks` (a copy stays on
    the CPU; every row walked costs the device more);
  - the device's other tests and parity checks (`setfix::device::`, 18). The random parity families had failed since
    decision 0229: past the bound head the CPU counts one witness, the device every row where its witness stage does
    not take the rest. They compare counts before each rule's tail (`setfix::step_tails`) and rows whole. Their
    generator no longer compiled (`g.pick` of a `&str` array inferred as `[str]`).
