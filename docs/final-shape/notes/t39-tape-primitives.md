# t39: tree primitives on the tape, and optimizing the credible candidates

## Intro

Rules ask the tape tree questions: "the function this call is in", "is this node inside a class",
"does this block contain a call", "the child named `body`". This note does three things:

- lists those questions by how often rules write them;
- says how each is answered on the tape itself, with nothing stored beside it;
- plans the next round of work on the three candidate tapes still standing.

t38 has the candidates and how they were measured.

Decided (2026-11-14):

- **The tape answers on its own.** Frequent kind sets (functions, classes, blocks) do not get stored
  answers. A few may be stored later, but the design must not count on them: memory is tight, and
  answering from the tape with very few stores is part of the tape's point.
- **The candidates are XF, MF and BE.** XF is the folded explicit tape with lengths (t38 E10, third
  round), MF is XF with predicted positions and gap ends, and BE is balanced parentheses with
  Elias–Fano positions. The rest of the B family (B, BM, BH) is retired, and so are X, M and XE: on
  every machine, one of XF, MF or BE is at least as good on both size and speed.

## 1. What rules ask

The count is of non-comment lines in the general rule packs and libraries (1,072 files) and in the
data-layer rules (140 files), under `docs/rule-syntax/`. Counts come from grep, so they are
approximate.

| question | general / data-layer lines | written as |
|---|---|---|
| child by field: `field(p, "name", c)` | 2,623 / 953 | base relation |
| child by index: `child(p, c, i)`, mostly `i = 0` | 1,712 / 510 | base relation |
| skip wrappers: `unparen`, `deparen`, attribute chains | about 960 / 290 | recursive library rules, downward |
| text equal to a constant, or matched | about 730 / 450 | `text`, `match` |
| `within(n, a)`: a is any ancestor | 314 / 139 | built-in axis |
| `nearest(n, a, S)`: the closest ancestor of a kind in S | 293 / 82 (`within_stop` 22) | built-in axis |
| `inside(n, a, S)` and `has(a, n, S)`: an ancestor, or a descendant, of a kind in S | 122 + 91 / 29 (about 100 negated) | built-in axes |
| document order: `earlier`, `visit_index`, `precedes` | about 120 / 140 | joins |
| siblings, lines, depth | 50–130 each | various |
| the outermost ancestor of a kind; the last child | about 10 each | aggregates, small recursions |
| lowest common ancestor; the path from the root | none | |

The axes are engine built-ins, "one tape pass per axis and set" (`lib/core.rr`). The engine
evaluates a rule a set at a time. So "the function of every call" is asked for every call at once,
not one call at a time. Each primitive therefore has two forms:

- **bulk:** the answer for every node of a file, in one pass;
- **point:** one node's answer.

## 2. How each is answered on the tape

| primitive | bulk | point |
|---|---|---|
| `nearest`, `inside`, `within_stop` | one preorder sweep, keeping a stack of the open S-nodes; only an S-node's subtree end is read | climb parents until one is in S |
| outermost of S | the same sweep; the answer is the bottom of the stack | climb to the root |
| `has` | one sweep: a node has one if an S-node comes after it before its subtree ends | read the subtree's codes in order until one is in S |
| `field`, `child` | inside the children walk | walk the children, comparing the field in each code |
| `unparen` | | first child, while the kind is a wrapper |
| `earlier`, `visit_index`, `precedes` | free: a node's id is its preorder number (BE: its open position, also in order) | free |
| `within(n, a)` with both bound | | `a < n < subtree_end(a)` |
| `line` | merge the starts, in order, with the newline positions | rank of the start among the newline positions |

A set is a mask over the language's codes: each code is one (kind, field, flags, memberships). So
"is this node in S" is one code read and one bit test.

## 3. First results (E12, M5, one thread, best of three)

`tape16 prim <capture>` runs every primitive on F15, XF, MF and BE. Every answer is checked against
the tree: 0 wrong, on all three corpora.

Data-layer, ns a node for bulk and ns a call for point (instructions omitted):

| | F15 | XF | MF | BE |
|---|---|---|---|---|
| nearest, every node (fn) | 2.2 | 1.8 | 1.7 | 4.8 |
| outermost, every node (fn) | 1.9 | 1.8 | 1.8 | 4.8 |
| has, every node (fn) | 4.2 | 5.2 | 5.3 | 11.2 |
| nearest, one node (fn) | 30,760 | 154 | 133 | 279 |
| has, one node (fn) | (noisy) | 21.9 | 21.8 | 62.9 |
| field `name`, one node | (noisy) | 23.1 | 13.3 | 20.3 |
| unparen, one node | (noisy) | 2.4 | 2.2 | 4.5 |

The other sets and corpora fall within 30% of these. F15 has no parent, so its point questions run
on 1 node in 1,024. At that rate, fixed per-call costs dominate its smaller numbers, so those are
marked "(noisy)".

- **Bulk is the form to use.** A bulk `nearest` costs 1.7–2.9 ns a node on XF and MF, and 4.8–6.9
  on BE. A point `nearest` costs 90–160 ns on XF and 200–400 on BE: a climb of 20–40 levels, each
  one a dependent read. So the planner should ask in bulk whenever a rule asks for more than about
  1 node in 50.
- **BE costs 2.5–3× XF on every axis,** as it did on the single questions (t38).
- **Bulk `has` costs 5–7 ns a node on XF** because the sweep keeps every open node and reads every
  subtree end. Read backwards ("the next S-node after a", against a's end), it needs no stack.
- **Point `has` and point `nearest` need to skip.** Both scan or climb across nodes that cannot
  match.

### Second pass: reading `has` backwards, and a block summary (M3, quiet; data-layer)

The M5 was under heavy load, so these timings come from a quiet M3 (`target-cpu=native`). Every answer is right on all three corpora.

| ns | F15 | XF | MF | BE |
|---|---|---|---|---|
| nearest, every node (fn) | 1.9 | 1.4 | 1.4 | 4.2 |
| has, every node (fn), forward with a stack | 4.5 | 4.6 | 4.5 | 8.6 |
| **has, every node (fn), backwards** | 1.5 | **2.6** | 2.7 | **7.5** |
| has, one node (fn) | (noisy) | 14.1 | 14.1 | 47.5 |
| **has, one node, skipping with a 64-bit filter a block** | (noisy) | **8.5** | 8.4 | **12.0** |
| has, one node, skipping with an exact summary | (noisy) | 9.9 | 9.1 | 13.5 |
| nearest, one node (fn), climbing | (noisy) | 92.7 | 91.4 | 221.8 |
| field `name`, one node | (noisy) | 9.4 | 9.4 | 14.7 |
| unparen, one node | (noisy) | 1.9 | 1.9 | 4.1 |

- **Bulk `has` read backwards** ("the next S-node after a, against a's end") needs no stack: 2.6
  ns a node on XF instead of 4.6.
- **The block summary is 0.032 B a node as a 64-bit filter** (each code hashed to one bit, a word
  every 256 nodes). It takes point `has` from 14.1 to 8.5 ns on XF, and from 47.5 to 12.0 on BE. On
  BE the win is mostly that the scan reads codes by rank, so it never searches for a close. The
  exact summary (0.09–0.23 B a node) does no better than the filter.
- **Skipping backwards for point `nearest` was dropped.** Common sets (calls, strings) occur in
  every block, so it read thousands of nodes a call: 0.5–10 µs, against 67–93 ns climbing.
  Point `nearest` stays a climb, and the planner asks for it in bulk.
- `within_stop(n, a, stop)` is bulk `nearest(n, stop)` plus one comparison per answer: a is an
  ancestor of n, at or below the nearest stop node. `precedes` is "same parent, earlier id". Neither
  needs a pass of its own.

### Third pass: line, position lists, and what the summary was really worth (M5, impl/sqlite)

These ran on impl/sqlite on the M5 under load, so compare columns, not absolute numbers. Every
answer is right.

| | XF | BE | tape part, B a node |
|---|---|---|---|
| line, every node (merge with the newlines) | 4.2 | 21.4 | 0.19 (Elias–Fano of the newline offsets) |
| line, one node (rank) | 27.9 | 56.8 | (the same) |
| has, one node: codes by rank over the subtree's interval | 18.9 | **23.0** | 0 |
| has, one node: by preorder steps (the first form) | 16.4 | 46.1 | 0 |
| has, one node: rank interval, skipping blocks by an exact summary | 17.6 | 22.9 | 0.09–0.23 |
| has, one node: per-kind position lists (a successor search per kind) | 33.6 | 44.0 | **2.39** |
| every node of a set: code sweep, a node | 1.5–1.9 | 1.5–1.9 | 0 |
| every node of a set: per-kind position lists, a node | 0.0–0.2 | 0.0–0.2 | 2.39 |
| every node of a set: sweep skipping by the exact summary, a node | 0.2 (classes) to 2.0 | the same | 0.09–0.23 |

- **`line` needs a 0.19 B a node index:** the newline offsets as Elias–Fano. A node's line is then
  a rank. In bulk, starts never decrease in preorder, so it is a merge. No source is needed.
- **What the second pass credited to the block summary was the loop's shape.** On BE, point `has`
  wins by scanning codes over the subtree's rank interval: it never searches for a close. That
  needs no summary. The summary adds nothing for `has`. As a 64-bit filter it skips nothing, because
  a block holds 40–60 distinct codes, so the filter is mostly ones. So the filter is dropped.
- **The exact summary pays only for very rare sets:** "every class", at 0.06% of nodes, is 7×
  faster. Every other set occurs in most blocks. It stays an option for later.
- **Per-kind position lists cost 2.39 B a node,** as much as the whole BE tape. They make "every node
  of a kind" 10× faster than a 1.5–1.9 ns sweep, and point `has` slower: a set spans several kinds,
  so it takes several searches. Under the decision above, they are not part of the tape. The sweep
  is the thing to make fast (step 3: bit-packed codes compared a block at a time).

### Step 3, first item: one record a node (XR) does not pay (M5, impl/sqlite)

XR keeps XF's four fields (code, slot, start above the block's least, length) side by side, one
record a node, each field at its block's own width, with each field's exceptions after the
records. A random read then touches one cache line instead of four. Every answer is right. Run on
the M5 under load, so compare columns:

| | XF | XR |
|---|---|---|
| bytes a node | 4.59 | 4.50 |
| code at random, ns | 1.5 | 3.1 |
| subtree end at random | 3.8 | 4.8 |
| children of every node | 8.9 | 10.6 |
| median of 121 rules / all 121 | 3.8 / 22.5 | 6.6 / 30.5 |
| nearest, every node (fn) | 2.4 | 3.8 |
| has, every node, backwards (fn) | 5.0 | 6.8 |
| has, one node (fn) | 16.8 | 24.8 |
| code sweep, a node | 1.7 | 2.9 |

- **XR is 1.3–1.7× slower everywhere, for 2% less space.** The primitives are sweeps: each reads
  one field of every node in order, and an array of that field alone streams. In a record, a
  sweep over codes reads all four fields' bits. A random read saves cache lines, but the field's
  offset in the record (a sum of widths) and its exception area cost more instructions than the
  misses saved: 40 against 28 a node for the code sweep.
- **Dropped.** The tape keeps one array a field, per block.

### Step 3, second item: 64 codes at a time, and the axes driven by the S-nodes (M5, impl/sqlite and data-layer)

XF, MF and BE keep the codes as one packed array at width w (9 bits for Python). 64 codes at width
w fill exactly w aligned words. So a sweep compiled once per width reads a group with constant
shifts, looks each code up in the set (one byte a code), and builds a 64-bit match mask. The
matches come out of the mask by trailing zeros, as in simdjson. Nothing is decoded or stored: the
mask lives in a register.

The bulk axes then read only the S-nodes:

- **`nearest` and `outermost`:** between two events (an S-node, or the end of an open one) the
  answer is constant, so it is written as a run. Only an S-node's subtree end is read.
- **`has`:** the nodes from one S-node up to the next, s, all have s as their next S-node. So each
  has one exactly when it is an ancestor of s. A climb from s, while the parent is at or after the
  S-node before, marks them, and each node is climbed over at most once.

Every answer is right on all three corpora (rerail: functions 0.3–0.6 ns on every axis). ns a node, M5 under load (compare columns):

| | XF before | XF now | BE before | BE now |
|---|---|---|---|---|
| every node of the set (fn), sqlite | 1.7 | **0.3** | 1.6 | **0.3** |
| nearest, every node (fn), sqlite | 2.2 | **0.5** | 5.8 | **0.7** |
| outermost, every node (fn), sqlite | 2.4 | 0.4 | 5.8 | 0.6 |
| has, every node (fn), sqlite (before: read backwards) | 4.5 | **0.5** | 15.4 | **1.0** |
| nearest, every node (call), sqlite | 2.7 | 1.2 | 6.8 | 1.9 |
| has, every node (call), sqlite | 4.6 | 2.1 | 14.2 | 3.1 |
| nearest / has, every node (fn), data-layer | 2.1 / 4.6 | 0.4 / 0.3 | 6.0 / 10.8 | 0.4 / 0.5 |
| nearest / has, every node (string, 24% of nodes), data-layer | | 2.3 / 3.1 | | 3.2 / 7.5 |

On a quiet M3 (`target-cpu=native`), 0 wrong on all three corpora, ns a node, before → now:

| | XF, data-layer | BE, data-layer | XF, rerail | BE, rerail |
|---|---|---|---|---|
| nearest, every node (fn) | 1.4 → **0.3** | 4.2 → **0.4** | 1.5 → 0.4 | 4.2 → 0.4 |
| has, every node (fn), against backwards | 2.6 → **0.3** | 7.7 → **0.4** | 2.6 → 0.4 | 8.1 → 0.5 |
| every node of the set (fn) | 0.9 → 0.3 | 0.9 → 0.3 | | |
| nearest, every node (string) | 2.5 → 2.0 | 5.3 → 2.6 | 2.1 → 1.3 | 4.9 → 1.7 |
| has, every node (string) | 2.6 → 2.2 | 8.7 → 5.2 | 2.6 → 1.5 | 8.5 → 4.1 |

- **The code sweep is 5× faster:** 7 instructions a node instead of 28. That is within 0.2 ns of
  the per-kind position lists (0.1 ns, 2.39 B a node), with no bytes stored. The first try, not
  unrolled, was 22 instructions: its shifts were variable. Unrolling the 64 steps made each shift
  a constant.
- **Bulk axes cost what their sparse sets cost.** For functions, classes and blocks (0.04–1.9% of
  nodes) the three axes cost 0.3–0.8 ns a node: the time to write the answer, plus the sweep.
- **BE gets most of the gap back.** It reads a subtree end, or a parent, only at S-nodes and their
  ancestors. So its 2.5–3× cost (t38, E12) shrinks to 1.0–2.4× on the bulk axes.
- **Dense sets are output-bound.** Strings are 13–24% of the nodes, and the S-node methods still
  win there, by less: 2.0 against 2.5 ns on the data-layer.
- So the bulk axes no longer need the plain sweeps. The plain sweeps stay in the lab as the
  baseline.

### Step 3, third item: a code table a block does not pay

Each block keeps a table of its distinct codes (at the file's width) and its codes as indexes into
the table, at the block's own width. Bytes a node for the codes:

| | one width a file (now) | a table a block of 64 | of 256 | of 1,024 |
|---|---|---|---|---|
| impl/sqlite | 1.12 | 1.20 | 1.05 | 0.99 |
| data-layer | 0.70 | 0.67 | 0.62 | 0.61 |
| rerail | 0.78 | 0.70 | 0.66 | 0.66 |

- **It saves 0.08–0.13 B a node, 2–3% of the tape.** A block of 256 holds 40–60 distinct codes, so
  its width is only 2–3 bits below the file's, and the table takes back most of the gain.
- **It costs the sweep its one width.** The 64-codes-at-a-time sweep would need the set translated
  into each block's table first. So it is dropped, and the codes stay one width a file.

### Step 3, fourth item: spans, and MF's gap end read a block at a time

`text` and `match` read a node's span (start, end). MF keeps the end as the gap to the start just
past the subtree, so one end is three dependent reads (the structure, the predicted start there,
the gap): 11.4 ns at random against XF's 5.4.

Read in bulk, a block at a time, each block's headers are decoded once and its values read in place:

- **XF:** start and length from their two blocks.
- **MF:** the predicted starts first, written into the answer. Then each end is the answer's start at
  the node past the subtree, minus the gap. That is a read of the answer, not of the tape.

The rightmost path ends with the file, so its few ends are read singly. Every answer is right.

| ns a node, instructions | XF | MF | BE |
|---|---|---|---|
| span, every node in order, a read a node (M3, quiet) | 3.6, 95 | 7.5, 204 | 32–35, 500 |
| the same, open nodes closed by the sweep (M5) | 5.6, 97 | 12.6, 181 | 47.6, 499 |
| **the same, a block at a time** (M5, under load) | **3.3, 52** | **6.1, 97** | 47.6, 499 |
| span of every call (M3), less the sweep | about 8 | about 12 | about 33 |

- **Read a block at a time, MF's ends cost half as much (12.7 → 6.1 ns).** The cost was each read
  decoding its block header again, not the random start: closing open nodes by the sweep alone saved
  nothing.
- **MF still costs about 2× XF on spans.** It does three reads a node where XF does two, and a
  multiply for the prediction. It is 13–19% smaller. So this is a real choice between size and
  speed, now about 3 ns a node.
- **BE's spans are its weak point:** 32–48 ns a node, because each end searches for the close. A
  range min-max tree (step 4), or a sweep over the parentheses closing nodes as it goes, is needed
  before BE can serve `text` in bulk.

### Step 4: BE, in bulk and at a point

- **Sampled select was in already.** `Ef::get` reads a sample every 64 values.
- **A top level of the range min-max tree would serve almost no search.** The second level
  covers 2,048 positions, about 1,000 nodes, and files average 2,100–3,400 nodes. Close searches
  that cross two or more second-level groups are 0.02% (sqlite), 0.07% (data-layer) of nodes,
  5–9 groups each on average (474 at most, in a file of 486,000 nodes). So it is not built. BE's
  point climb costs 2× XF's (data-layer: 266 against 131 ns) because of the near search inside each
  step, which a tree above would not touch.
- **Spans in bulk need no search at all.** One pass over the parentheses: an open pushes its node,
  a close pops it, and BE's ends are Elias–Fano in postorder, so the k-th close takes the next
  value of an in-order cursor over the ends. Starts come from a cursor over the starts. Every
  answer is right.

| span, every node, ns a node (M5, under load) | XF | MF | BE |
|---|---|---|---|
| a read a node | 3.1 | 6.5 | 26.2 |
| **a block at a time (BE: the parenthesis pass)** | **1.7** | **3.2** | **5.0** |

### Step 5: the bulk primitives on Metal (M3, quiet)

`tape16 prim-gpu <capture>` runs the kernels (`src/prim_gpu.rs`). They read each candidate's
one-buffer image in place, with one thread a group of 64 nodes by preorder rank, on one language's
files as one forest:

- **match:** the group's 64-bit mask of S-nodes.
- **nearest and outermost on XF and MF:** the thread climbs from its group's first node to the
  S-ancestors open there (its carry), then sweeps its 64 nodes with that stack.
- **nearest on BE:** a scan with block carries, since BE's climb is a search a level.
  - Each group walks its parentheses from its first open, found by a select over the rank
    directory. Its walk is a stack transformer: the lowest level it reaches, its net change, and
    the S-nodes still open at its end.
  - The transformers compose up and down a 64-ary tree, so each group gets the S-nodes open at its
    start, and answers the nodes it could not answer alone.
  - `outermost` follows each answer's chain of S-nodes.
- **has:** from each S-node, a climb that sets each ancestor's flag by an atomic exchange and stops
  at a flag already set, so the work is the number of nodes marked.

Every answer is right on all three corpora. ns a node, including the match. GPU runs vary by
about 2× between clock states (the match alone reads 0.021 or 0.056), so these are typical values
of single runs. CPU is one thread, from the S-nodes.

| data-layer Python (2.47M nodes) | GPU nearest | GPU has | CPU nearest | CPU has |
|---|---|---|---|---|
| XF, fn / call / string | 0.05–0.12 / 0.06 / 0.05 | 0.10 / 0.16 / 0.16 | 0.68 / 1.77 / 1.16 | 0.57 / 2.00 / 1.80 |
| MF | the same as XF | the same | the same | the same |
| BE, scan | 0.24 / 0.22 / 0.17 | 0.49 / 0.85 / 0.69 | 1.03 / 2.86 / 1.54 | 1.94 / 4.75 / 3.96 |
| BE, carry by climbing (the first form) | 2.46 | | | |

rerail (Rust, 1.41M nodes): XF and MF nearest 0.06–0.13, BE's scan 0.19–0.35, BE climbing 3.05.
sqlite (0.47M nodes) is too small to fill the GPU: XF 0.11–0.15 there.

- **On the GPU, the bulk axes cost 0.05–0.13 ns a node on XF and MF:** 5–30× one CPU thread.
- **BE's scan is 10× its climbing carry,** and 2–4× XF. BE's `has` still climbs with a search a
  level (0.3–0.85). The same scan would serve it: a node has one if an S-node opens before the
  walk's level falls below the node's.
- **Every kernel reads the tape in place.** The only other buffers are the match masks (one bit a
  node), the scan's summaries (144 bytes a group of 64, about 2.3 B a node, freed after) and the
  answers.

### Step 5, finished: `has` from counts, and spans on the GPU

- **`has` from counts, with no climb.** `has(a)` asks whether an S-node ranks after a and before
  a's close. With S(r), the number of S-nodes ranked below r, it is `S(close rank) − S(a + 1) > 0`.
  S comes from a scan of the groups' mask counts, up and down the same 64-ary levels.
  - XF reads the close rank from its slot.
  - BE's group walk has it free for nodes that close in the group (the opens so far). Only the few
    still open at the walk's end search for their close.
- **Spans on the GPU:**
  - XF: start and length.
  - MF: the starts first, then each end is the answer's start past the subtree, less the gap.
  - BE: each close's postorder number is its position less the opens before it, which indexes the
    ends.

### Step 6: everything on the seven machines (2026-11-14)

The pack (sqlite, data-layer, rerail captures) ran on:

- three Apple machines: an M1, an M3, and an M1 with a larger GPU;
- four x86 cloud VMs: Zen 3, Zen 4, a recent Intel server CPU, and an older Intel server CPU.

Each machine ran `inplace` (baseline and native builds, and Metal on the Macs) and `prim` (native).
The Macs also ran `prim-gpu`. **Every answer is right on every machine, corpus and candidate,**
and the VMs are deleted.

Data-layer, native build, ns a node (bulk) or a call (point). Bulk is functions; rerail is within
0.1–0.2 ns of these.

| | M1 | M3 | M1, larger GPU | Zen 3 | Zen 4 | recent Intel | older Intel |
|---|---|---|---|---|---|---|---|
| 121 rules, XF / MF / BE | 18 / 24 / 72 | 15 / 19 / 52 | 18 / 23 / 72 | 25 / 37 / 66 | 25 / 36 / 64 | 25 / 33 / 79 | 48 / 68 / 133 |
| code sweep, 64 at a time | 0.3 | 0.3 | 0.3 | 0.4 | 0.4 | 0.4 | 0.6 |
| nearest, bulk, XF / BE | 0.4 / 0.5 | 0.3 / 0.4 | 0.4 / 0.5 | 0.5 / 0.5 | 0.5 / 0.6 | 0.6 / 0.7 | 0.9 / 1.0 |
| has, bulk, XF / BE | 0.4 / 0.5 | 0.3 / 0.4 | 0.4 / 0.5 | 0.4 / 0.6 | 0.5 / 0.6 | 0.5 / 0.7 | 0.7 / 1.1 |
| span, bulk, XF / MF / BE | 2.3 / 4.4 / 7.2 | 1.9 / 3.4 / 5.2 | 2.3 / 4.4 / 7.2 | 3.5 / 6.5 / 8.7 | 3.2 / 6.1 / 8.0 | 3.4 / 6.1 / 9.2 | 6.7 / 13.5 / 20.0 |
| nearest, one node, XF / BE | 119 / 281 | 97 / 227 | 121 / 281 | 124 / 301 | 125 / 303 | 147 / 419 | 196 / 695 |
| field `name`, one node, XF / BE | 14.5 / 20.5 | 9.3 / 14.1 | 14.4 / 20.2 | 18.9 / 26.2 | 17.2 / 23.1 | 19.7 / 32.0 | 35.6 / 59.2 |

Bytes a node (the same everywhere): XF 4.15, MF 3.37, BE 2.43.

GPU, data-layer Python, functions, ns a node, including the match (single runs, ±2× by clock
state):

| | M1 | M3 | M1, larger GPU |
|---|---|---|---|
| nearest, XF / BE (scan) | 0.67 / 2.73 | 0.05 / 0.24 | 0.34 / 0.74 |
| has, climbing, XF / BE | 0.39 / 1.63 | 0.10 / 0.49 | 0.27 / 0.89 |
| has, from counts, XF / BE | 1.66 / 1.65 | 0.16 / 0.24 | 0.35 / 0.57 |
| span, XF / MF / BE | 1.55 / 3.03 / 2.13 | 0.08 / 0.16 / 0.33 | 0.33 / 0.61 / 0.81 |

- **The bulk axes are 0.3–0.7 ns a node on every recent machine, on all three candidates.** BE is
  within 0.1–0.2 ns of XF in bulk (0.4 on the older Intel), as on the M5. Only the older Intel is about 2×
  slower.
- **The gap between the candidates is now point reads and spans:**
  - BE's point climb is 2.3–3.5× XF's;
  - BE's spans are 2.5–3× XF's, and MF's 1.8–2×;
  - over the 121-rule mix (a per-read mix, not bulk), BE is 2.6–4× XF.
- **On the GPU, the M3 is 5–10× the M1.** On the M1's small GPU, the scans (BE's `nearest`,
  `has` from counts) cost more than climbing: few cores, and the levels' serial steps. `has` by
  climbing stays the better form there. On the M3 the scans win for BE.
- **x86 needs nothing special.** The sweep, the bulk axes and the spans run at about Apple's speed
  on Zen 3/4 and the recent Intel. The recent Intel's point reads on BE are slower (`findclose`, as in
  t38).

## 4. What using the bulk primitives takes in the engine

Surveyed in `native/src/code_rule_dsl` (2026-11-14).

**What the engine does today:**

- **The rules never read the tape.** At load, `facts/core.rs` (`from_image`, `from_tape`) decodes
  each document into `Facts`: per-node vectors (kind, parent, start, end, field, children) and a
  per-kind node index (`facts/core/postings.rs`). Every base view (`node`, `child`, `field`, `tape`,
  `line`) reads `Facts` through `Projected` (`facts/vtab/syntax.rs`). Node ids are granule ids, not
  preorder numbers.
- **The kind-set axes are one source,** `Within` (`facts/vtab/syntax/within.rs`, 535 lines). The
  set is a table of bools a code. The default is a point lookup a key: a climb or a subtree walk,
  memoised. `solved()` is a bulk pass for `nearest` and `inside`, asked from n only. `has`, the
  other direction, and `within` have no bulk form.
- **The planner picks bulk or point per axis** (`Within::expect`). It goes bulk once expected keys ×
  150 ns exceed nodes × 12 ns.
- **`inside` and `has` are relations of pairs,** not yes or no: `inside(n, a, S)` gives every
  S-ancestor, and `has(a, n, S)` every S-descendant. Only `!has(b, _, S)` asks "any".
- **Sets of nodes and derived kind sets** are lowered to recursive rules over `child` or `tape`
  (`logic/rr/lower.rs`, `node_axis`, `derived_axes`), run by the general fixpoint.
- **The ast matcher and the Metal kernels** read format-15 tape documents directly (about 590
  `TapeDoc` calls in 74 files), and build their own per-kind anchor indexes.

**The rework, by size:**

1. **The axes on `Facts` (small: `within.rs`, about 500–1,000 lines).** Bulk `nearest`, outermost
   and `has`, from either side, from the S-nodes. Bulk `inside` and `has` as pairs: for each S-node
   its preorder interval, so every pair comes from one interval. The planner's constants become
   about 0.3–1 ns a node for bulk, so bulk wins once a rule asks for more than about 1 node in 300.
   The sweep runs on `Facts`' kind array. Answers keep `Projected`'s numbering, or are mapped to it.
2. **Sets of nodes and derived kind sets on the same code (moderate, `lower.rs`).** The S-node
   methods need only the S-nodes in preorder, not a code mask. A derived relation of nodes, sorted,
   drives the same sweep, in place of the recursive climbs.
3. **The rules reading the tape in place (large: `facts/core.rs`, `Projected`, the base views).**
   This is what the tape's design asks for. `Facts` is a decoding of the tape: a per-node copy and a
   per-kind index, the stored answers that 0156 and the decision above rule out. The views would
   read a `Reader` over the format-16 tape (code, structure, span, field in the code), with ids as
   preorder numbers. Granule ids and narrowed documents would map onto preorder ranges. Most base
   views go through `Projected`, so that is where the work is.
4. **The ast matcher and Metal (largest, and separate).** About 590 call sites and the kernels'
   node packing. The rule axes do not need it.

**Where the work goes:**

- (1) alone gets the bulk speed on today's `Facts`, but keeps the decoding.
- (3) is the one that removes the copy.
- (1) and (2) carry over to (3) unchanged if they are written against a `Reader`-like interface
  over `Facts` from the start. So (1) should be written that way.

## 4b. Against ast-index-spike's tape

The spike's tape (prior-work design index, topic 1; DESIGN S012, S039, S045–S047, S071) is a fixed
6-byte record a node:

- `kf`, 2 B: kind (10 bits) and field (6 bits), local to the language;
- `par`, 2 B: the distance back to the parent;
- `size`, 2 B: the subtree's size.

A value that does not fit in 2 B is a sentinel 0xFFFF and an entry in a side list (about 0.3%
of parent distances). Spans are a separate stream of two varints a node (zigzag start delta, length), with a
checkpoint every 16 nodes. Depth, child position and sibling order are recovered, not stored. Its
speed was measured inside SQL, where row machinery, not the encoding, dominated (topic 19, S152:
110 ns a node touched, of which decode is 38). So its numbers compare designs, not this lab's
nanoseconds.

The shape is the same as XF's: preorder, a subtree's extent, a node's id its preorder number.
Descendant tests are intervals, children are `i + 1` then `+ size`, and nothing is recursive (topics
2 and 7). The differences are in how each part is stored:

| operation | spike's 6-byte record | XF / MF | BE |
|---|---|---|---|
| code of one node | one aligned 2-byte load: the cheapest form | one bit extract at the file's width: about the same | the same, by rank |
| is a node in S | kind and field apart: a kind test and a field test | one bit: the code is kind, field, flags and memberships | the same |
| every node of S (the sweep) | 2 bytes of every 6, strided: about 5× the bytes of XF's packed codes; no 64-at-a-time match without gathering | packed codes, 64 at a time: 0.3 ns a node | the same |
| subtree end | one aligned load; a side list search past 65,535 | one PFOR read (a header and an extract); an escape names its exception, so no search | a close search: 2–3× XF |
| parent | one aligned load (`par`) | one read: the slot of the node before | an enclose search: 2–3× XF |
| children, siblings, depth | `i + 1`, `+ size`; depth recovered | the same | the same in bulk; a search a step at a point |
| span of one node | the checkpoint, then up to 15 varints decoded in order, branchy | XF: two reads (start, length). MF: three (the gap needs the next start) | two Elias–Fano selects |
| spans of every node | a varint stream, in order: cheap, byte-serial | a block at a time: 1.7–2.3 ns (XF), 3.2–4.4 (MF) | the parenthesis pass: 5–7 ns |
| bulk nearest, outermost, has | not a primitive: recursive climbs or descents in SQL (130 µs a row as CTE steps; S037) | from the S-nodes: 0.3–0.5 ns a node | the same, 0.4–0.6 |
| nodes of a kind | kidx postings beside the tape (1.6–2.4 B a node; the anchor index) | the sweep; per-kind lists priced (2.39 B a node) and left out | the same |
| on the GPU | not designed (topic 19) | the kernels above, one thread a group of 64 | the same, with the carry as a scan |

What the comparison says:

- **The spike's record is the fastest single read.** Byte-aligned fixed fields need no header and
  no extract. Its subtree end and parent are one load, like X's (t38), and somewhat cheaper than
  XF's PFOR reads. Only its rare overflow, a side-list search, is slower than XF's indexed escape.
- **XF is about half the size or less, and its sweep is the better bulk primitive.**
  - The spike's code and structure take 6 B a node against XF's 1.7–2.1. With spans and flags its
    logical tape was 8.66 B a node (topic 1, "Stored size"), against XF's 4.15 and MF's 3.37 on
    data-layer, everything included.
  - Its codes sit 2 bytes in every 6, so a code sweep pulls in whole records: for 64 nodes, 384
    bytes against XF's 72, and no 64-at-a-time match. That is the layout XR measured: 1.3–1.7× slower (step 3).
- **Its span read is the weak point:** a random span decodes up to 15 varints. XF reads two values
  in place, and MF three. Read in order, both are cheap.
- **Its record gives repeated bits for repeated structure.** `par` and `size` are relative and
  `kf` is fixed. So two equal subtrees are equal bytes anywhere, which zstd and a second hash can
  use. In XF and MF a block's widths and bases (and BE's Elias–Fano positions) differ from block to
  block, so equal subtrees in different blocks can differ in their bits. Of format 16's
  requirements, this is the one where the spike's choice is better.
- **It assumes small files:** 2-byte fields with side lists. Format 16's PFOR widths grow with the
  values, and positions above 4 GB fit.
- **BE is the spike's "succinct BP, about 2 bits a node, needs rank/select, not built" (S039).**
  Built here, it is the smallest tape (2.43 B a node). It matches XF in bulk, and is 2–4× slower
  wherever a point read searches.
- **The spike answered axes through kidx postings and recursive SQL.** Format 16 answers them in
  bulk from the bare tape, which is why postings are not part of it (the decision at the top).

## 5. The plan

In order. Each step is measured on all three corpora, every answer checked.

1. **Primitives, finished.**
   - Done: bulk `has` read backwards; the block filter for point `has`; `within_stop` and `precedes`
     reduced to `nearest` and the parent.
   - Done: `line` (0.19 B a node); point `has` by rank interval; the block summary and per-kind
     lists priced and left out.
   - The block summary, as first planned: A block summary of which codes occur would let point `has` skip a
     block without S, and point `nearest` skip backwards to the last S-node whose subtree covers
     the node. It costs 0.03 B a node as a 64-bit filter a block, or 0.2 B as an exact mask. It is
     generic, not tied to any set, so it can be part of the tape. The same summary lets a sweep
     for a rare kind skip blocks.
   - The costing weights become these rule counts in bulk form, not the old per-read mix.
2. **Per-kind position lists, measured with and without** (about 1–1.5 B a node). They serve the
   rare-kind sweep, point `has` (a successor search in the subtree interval) and "every node of
   kind K". They are the "stored answers" the decision above is wary of, so they are an option to
   price, not part of the design.
3. **Layout.**
   - ~~Interleave each block's arrays~~: measured as XR, 1.3–1.7× slower; dropped.
   - Done: the code sweep 64 codes at a time, and bulk `nearest`, `outermost` and `has` driven by
     the S-nodes (0.3–0.8 ns a node for sparse sets).
   - ~~Narrower codes a block~~: saves 0.08–0.13 B a node (2–3%) and breaks the one-width sweep;
     dropped.
   - Done: spans a block at a time (MF's end 12.7 → 6.1 ns a node, XF 5.8 → 3.3).
4. **BE.** Done: spans by one pass over the parentheses (26 → 5 ns a node). Sampled select was in
   already. A top level of the range min-max tree was priced from the searches it would serve
   (0.02–0.07% of nodes) and is not built.
5. **Metal.** Done: match, nearest, outermost and has as kernels on XF, MF and BE, with BE's carry
   as a scan (0.05–0.35 ns a node); `has` from counts, with no climb; spans.
6. **Rerun everything on the seven machines.** Done (2026-11-14): every answer right everywhere.
