# 0694. A relation keyed by one node is read by the node

Status: decided (the user, 2026-11-25), being built: step 1 built and measured.

## Intro

Note t57 compares W (6970a3e539) with the head on the same corpus, and names ten expansions (E1–E10) between them.
E7 is this one: a name read passes a dozen relations keyed by one node (`rf_site`'s chain of tests, `wstart`,
`rf_in`, the `rf_*` `via`s), and each is a relation of rows, joined by an index probe. The cost cases count 122 join
units a name read (after E4) against a bound of 40.

A spike beside the engine (`rerail_blocks::node_keyed`, 26cf41db0b) measured the other form on the same rows. Relax,
one thread, 2 M tests over 200 k nodes, against a frozen relation probed by its index on the node:

| Read | Node-keyed | Index probe | Bytes |
|---|---|---|---|
| A test of a bound node (a set) | 3.2 ns | 24.7 ns | 35 times fewer |
| A node's value (one a node) | 10.7 ns | 58.9 ns | 4.5 times fewer |

The user's direction, 2026-11-25, on the plan that followed the spike: "do all of this", the plan being

1. Planner: pick this form for a relation whose declared or inferred key is one node column of one document.
2. Writes: a rule writes such a relation's rows into the column, and a broken key falls back to a relation.
3. Reads: setfix tests a bound node with one load instead of an index probe.
4. Other readers: the same read written once in CLIF for the JIT and Metal (decision 0310).

starting with `rf_site`'s chain of tests and `wstart`.

## What it can reach

A profile of the head on all (m1max, `RERAIL_GPU=none`, t57) splits its CPU so:

| Part | Share |
|---|---|
| Joins (the walker, its probes and candidates) | 53.6% |
| Tape views, the matcher wave included | 37.9% |
| Component relations' runs | 9.7% |
| Writes (fresh rows, freeze, settle) | 9.5% |
| Derived inputs' probes | 2.8% |

(The parts overlap where a join reads a view.) Most of the walker's tests (`tests_keys`) test views, not derived
relations; a view is a read of the tape already (decision 0353) and gains nothing here. So E7 reaches at most about
22% of the head: the derived relations' probes, their writes, and the join steps they feed. The rest of W's gap is
in the views' reads and the walker's own work, which this decision does not change.

## Decision

A relation keyed by one node column is held and read by the node: a node-keyed index in the relation's block, and
later the relation's storage itself.

### The forms

The spike's three:
- **A set**: a bit a node. A test of a bound node is one bit.
- **One value a node**: a `u32` a node (a node's rank among the values for wider ones). A second value for a node
  breaks the key: its writer is told, and the relation is held as rows instead.
- **A fan**: a node's values grouped by offsets, for a key that is not unique.

A node's value is its document's key (the high 32 bits) above a stable id (the low 32 bits): its granule's slot
above its index in the granule (`packed::ID_SPLIT`, 18 index bits), not its dense local id. Reaching the local id
from the value is the numbering's rank work, which costs about what the probe does. So every form is keyed by the
value as it is: by document, then by slot, then by index.

**Measured on the corpus (step 1):** a relation keyed by node holds a small share of each document's nodes, a few
in each of many granules. `rf_site` holds 79,415 rows, about 26 a document. A bit an index over each document's
slots took more bytes than the hashed index for 1,746 of 2,118 keys, `rf_site`, `nameref`, `look` and
`rf_bound_at` among them. So a set holds each document in the smaller of two forms:
- **bits**, where its values are dense in their slots: a slot table, then a bit an index;
- **lows**, otherwise: its distinct low halves, sorted (4 bytes a value).

A document takes bits when they are at most twice its lows' bytes. Neither form is larger than a hashed index, so
every key of one node column has one.

### It is not a copy (decision 0353)

Each form is an index in the relation's block, or (step 2) the relation's own storage, written once where the rule
computes it. No reader reads a second structure of the engine's instead of the block.

### The steps

1. **Reads.** A test of a derived input by one bound column reads the input's node-keyed index
   (`Relation::try_add_node_bits`, the index kind `NODE_BITS` in its block's index directory). It is built at the
   key's first test, under the block's index lock, beside the hashed index. The document is found (the last one
   and the one after it are tried first). Then either its slot's entry and one bit are read, or its lows are
   searched. No candidate row is read. `RERAIL_NODE_BITS=off`, a measurement aid read at a key's first test,
   probes as before.
   - Where it applies: a step of the walker (and a test asked directly, `cands::holds`) that binds nothing (or is
     negated), tests no column pair, and is neither fused nor ranged. It also applies only past the first step and
     not to the step whose rows are a delta's.
   - Where it does not: the input is a view, has dead rows, or lies in several parts. The step then probes as
     before.
2. **Values.** A step that binds one value a node (`wstart`, `rf_in`) reads the value form: the node's rank in the
   set's bits gives its value's place.
3. **Writes.** A rule whose head's key is one node column writes its rows into the form as it derives them. A
   broken key (a second value) falls back to a relation of rows, its rows so far moved there once.
4. **The planner.** It picks the form for a relation whose declared or inferred key is one node column of one
   document, and prices a test by it as one load.
5. **The other readers.** The same reads, written once in CLIF (decision 0310), for the JIT and the Metal kernels.

### Measured

Each step is measured W against the head on all, as t57's runs are, and its result is written here.

**Step 1** (m1max, the head with `RERAIL_GPU=none`, one build, two runs each, load 5.5–6.5):

| | CPU | Instructions | Peak |
|---|---|---|---|
| Node bits | 137.5–137.8 s | 1,528–1,531 G | 2,099–2,111 MB |
| `RERAIL_NODE_BITS=off` | 140.6–141.2 s | 1,578 G | 2,094–2,098 MB |

That is 3.0% fewer instructions and 2.3% less CPU, with the same findings' rows. It is about the whole of the
derived inputs' probes' share (2.8%). The rest of E7's reach is in steps 2–4.

## Consequences

- An index kind the hashed-index and order finders do not find (`find_index_from` asks for `HASHED`, `find_order`
  for `ORDER` or `ORDER_ROWS`). Its chunks are given back with the relation's other indexes (`forget_indexes`).
- A test by bits reads no row, so a run's join units for that step count the test, not candidates.
