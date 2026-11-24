# 0667. A submission's closures are one launch: a work queue over their components

Status: done (2026-11-24).

## Intro

The GPU lane computes a graph's closure (`Kernel::Closure`, the reach of every node as a row of bits) over its
condensed DAG: the host finds the strongly connected components, and the device fills each component's row from its
successors' rows. Until now it did that a level of the DAG a launch: `cl_init`, then one `cl_level` for each level past
the first, then `cl_gather`. A chain of 300 nodes took 302 launches; a submission's closures took the sum of theirs.

The engine sends its closures as a batch (`setfix::closure`: one launch for each piece of dense components), and
`gpu::tests::closure_is_the_reference`, a batch of 15, failed under decision 0601's budget: "ClLevel: the 200
launches of its evaluation spent; ClGather 13, ClInit 14, ClLevel 173". Past the budget is a hard failure, and the fix
is the launch count, not the budget.

## What was built

### One walk, a work queue (`cl_walk`)

The three kernels are one, `cl_walk`, in `accel/src/gpu/relational.uglykl`. It follows `label_phase`'s pattern
(decision 0540 step 6, `<ukl/workqueue.h>`):

- An item is a component, and a threadgroup of 64 threads handles one item. It sets its own bits: every node its
  members' edges reach outside it, and its members when it is cyclic. It ORs in its successors' rows, read
  coherently, since other groups wrote them. It copies the row to each member's row of the output.
- It then counts down each predecessor's successors left. The group that takes a count to zero pushes that
  predecessor onto the queue (the continuation), after a device fence over its writes.
- The seeds are the components with no successor, taken from a counter. A group that finds nothing to take ends. No
  group waits for another, and the grid is one group a component, never sized by residency.
- The queue's capacity is the continuations (the components past the seeds) and one slot a group, as a take may give
  one up.

An item is pushed only once its last successor is done, so the successors' rows it reads are final. That is the
level order without the level boundaries: a long chain is one group's work in turn, and wide parts run in parallel.

### Every closure of a submission together

`Planner::jobs` plans a submission's closures together (`Planner::closures`). Their components are numbered over all
the graphs, each row at its own place in one row buffer and each graph's output at its own place in one output buffer
(`Read::Bits`'s new `at`). One `cl_walk` launch does them all. So a submission's closures are one launch however many
there are and however deep their graphs.

The device reads only arrays the planner made from the condensed DAG (components' rows, successors, own bits,
predecessors, members' output rows). The caller's edge columns are no longer bound to a kernel.

### A walk that left work is a failure

A job's failure words (`Bad`, which replaced the histogram's one flag) name the words the device leaves nonzero on
a failure, and the error. The walk's are the queue's active count and its overflow: a run that left an item pushed
and not done, or ran out of slots, fails, rather than return rows short of their bits.

## Tests

- `closure_is_the_reference` (accel): its 15 closures, a chain of 300 levels among them, are the reference's, in one
  dispatch (asserted). It passes on Metal (M1 Max) and on the CPU's device (a Linux host), with the 14 tests of
  `gpu::tests`, `concurrent_submissions_are_exact` and the mixed batch's one command buffer included.
- The engine's closure tests (`rerail-tests-logic`, the 14 named for closure, reach and walk along) pass on Metal
  (M1 Max).
