# 0723. Every stage is a prepare and an apply

Status: decided (2026-11-27), by the user: "for the filter stages (and in fact all of the phases), I want each to
have two parts: a planner-driven preparation part. This could do things like create a bitmap by using a batch query
of things that would be otherwise point queries in an inner loop, sort its inputs, create bloom filters / hash
maps, or whatever; then, the actual filter, which should be as cheap as fuck if the former part was done correctly.
Same for mapping stages." Then, on the design page that took it (the order of work below), "go ahead". Being built.
Follows decision 0706 (executors by step kind, indexes built ahead and shared), 0694 (node bits) and 0353 (blocks
never copied).

## Intro

Decision 0706 batched five step kinds: node reads, membership tests, aggregates, seed scans and keyed probes. Each
step moved CPU by 0 to 4% on `all`. Its records give one reason each time: the batches took the rows, and the reads
under them cost what they did. Batching changed how the walker visits rows; every row still made the same point
query (its node's document and local id found, a rank select, a view's row read).

So each stage is two parts:

- **The prepare**, chosen and priced by the planner, which turns the point queries an inner loop would make into one
  batch query: a bitmap, a batch's keys sorted and merged against an order, a filter or hash set, a batch's nodes
  resolved to their documents together. It runs at one of two times:
  - **ahead**: keyed by constants or a whole relation, built once before the strata that read it, as an index in its
    relation's block (0706's catalogue: `Access`, `Available`, built ahead and shared);
  - **per batch**: keyed by the batch's bound values; its results are scratch of that computation, read by no other
    reader, so not a copy (0353).
- **The apply**, the filter or map over a chunk: with its prepare done, a bit test, a compare, or a read at an index
  the prepare computed. No search, hash or call per row.

A stage is done when the profile of its apply has none of `node_hint`, `granule_of_low`, `select`, `read_one_at` or a
hash probe under it.

## Order of work

1. View bits by a constant (`kind`; then `field` and `child` by a constant), ahead, applied as bit tests in the
   walker and the fused chunks.
2. The merge of the uglykl line (`rerail-reads`, `lanes`, `stage_lanes`, t58, 0707), by invariant.
3. Per-batch node resolution: a batch's nodes sorted, their documents found in one pass, their columns read through
   `Doc::of`. For view probes that bind and node maps.
4. Computed relations' probes as sorted merges (t58 B), the candidates as one stream (`stage_lanes`); anti-joins on
   packed-key bitmaps.
5. Value maps computed once a distinct argument tuple.

Each is measured on `all` against the head, with the same rows, and written here.

## Step 1, as built: a view's tests by a constant read the constant's bits

- `blocks::relation::node_bits_chunk`: 0694's node bits over any values, in a block (the build `try_add_node_index`
  now calls), and `NodeIndex::in_chunk`, its read in place.
- `logic::access::Kind::ValueBits(node, col, value)`: a view's nodes whose column holds a constant. It pays by its
  tests (a view's row read by node, `VIEW_READ_NS` 150 ns, against a bit, 12) over the nodes it holds.
- `Source::build_value_bits`, `value_bits`, `value_bits_gen` (facts_base): the `node` view's source builds a kind's
  bits from its rows by kind (`KindRows`, `Batch::by_kind`) into the batch's block (`Batch::kind_bits`). A layered
  source (tape values admitted, decision 0431) gives its base's bits with the document keys its parts hold rows of
  (`others`, from `Parts::by_key`, kept until more values are admitted): a node of one is read as without the bits.
- setfix: a test of a bound node against a constant (`kind(n, K)`, `!kind(n, K)`, with no binds) is the
  catalogue's `ValueBits` need, observed with the run's other indexes, and built by the level's build ahead
  (`access::prefetch`, the source of a view held in place by its name, `hold_view_in_place`). The walker
  (`enter`, before a node read's batch), a fused chunk's tests (`cands::holds`) and its lookups that bind nothing
  (`cands::look`) answer it by the node's bit (`cands::value_bits_test`), the bits found once a walk a step.
  `RERAIL_VALUE_BITS=off` reads the rows as before (a measurement aid).
- `RERAIL_PROF`: "view bits" counts the walks' steps that found bits, the tests they answered, and the nodes built.
- Tests: `kind_tests_by_a_bound_node_read_the_kinds_bits_built_ahead` (tests_corpus: `kind(n, "identifier")` and its
  negation over 80 files in chunks, the rows the source's, the bits answering more than 100 tests),
  `a_views_bits_by_a_constant_pay_by_their_tests_against_the_constants_nodes` (logic-core).

On `all` (m1max, `RERAIL_GPU=cpu`, profiled, one build, off and on alternately, twice; the findings' rows the same):
17.1 M tests answered by a bit, bits built over 6.7 M nodes. Instructions 1,269 / 1,272 G off, 1,259 / 1,256 G on
(−1.1%); CPU 120.2 / 121.1 s off, 121.5 / 117.4 s on, within the load's spread (13–25). W: 57.5 s, 578 G.

About 800 instructions saved a test, so the tests were dear one by one, but they are a small part of the run.
