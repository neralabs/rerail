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

## 3. First results (E12, M5 Max, one thread, best of three)

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

### Second pass: reading `has` backwards, and a block summary (M3 Max, quiet; data-layer)

The M5 was under load 80–160, so these timings come from the M3 Max (load 3, 94% idle,
`target-cpu=native`). Every answer is right on all three corpora.

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

On the M3 Max (quiet, `target-cpu=native`), 0 wrong on all three corpora, ns a node, before → now:

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
| span, every node in order, a read a node (M3 Max, quiet) | 3.6, 95 | 7.5, 204 | 32–35, 500 |
| the same, open nodes closed by the sweep (M5) | 5.6, 97 | 12.6, 181 | 47.6, 499 |
| **the same, a block at a time** (M5, under load) | **3.3, 52** | **6.1, 97** | 47.6, 499 |
| span of every call (M3 Max), less the sweep | about 8 | about 12 | about 33 |

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

### Step 5: the bulk primitives on Metal (M3 Max, quiet)

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

- the M1 Mac mini, the M3 Max and the M1 Max;
- four GCP spot VMs: Zen 3 (EPYC 7B13), Zen 4 (EPYC 9B14), Sapphire Rapids (Xeon 8481C), and
  an older n1 Xeon at 2.2 GHz.

Each machine ran `inplace` (baseline and native builds, and Metal on the Macs) and `prim` (native).
The Macs also ran `prim-gpu`. **Every answer is right on every machine, corpus and candidate,**
and the VMs are deleted.

Data-layer, native build, ns a node (bulk) or a call (point). Bulk is functions; rerail is within
0.1–0.2 ns of these.

| | M1 | M3 Max | M1 Max | Zen 3 | Zen 4 | SPR | n1 Xeon |
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

| | M1 | M3 Max | M1 Max |
|---|---|---|---|
| nearest, XF / BE (scan) | 0.67 / 2.73 | 0.05 / 0.24 | 0.34 / 0.74 |
| has, climbing, XF / BE | 0.39 / 1.63 | 0.10 / 0.49 | 0.27 / 0.89 |
| has, from counts, XF / BE | 1.66 / 1.65 | 0.16 / 0.24 | 0.35 / 0.57 |
| span, XF / MF / BE | 1.55 / 3.03 / 2.13 | 0.08 / 0.16 / 0.33 | 0.33 / 0.61 / 0.81 |

- **The bulk axes are 0.3–0.7 ns a node on every recent machine, on all three candidates.** BE is
  within 0.1–0.2 ns of XF in bulk (0.4 on the n1), as on the M5. Only the old n1 Xeon is about 2×
  slower.
- **The gap between the candidates is now point reads and spans:**
  - BE's point climb is 2.3–3.5× XF's;
  - BE's spans are 2.5–3× XF's, and MF's 1.8–2×;
  - over the 121-rule mix (a per-read mix, not bulk), BE is 2.6–4× XF.
- **On the GPU, the M3 Max is 5–10× the M1.** On the M1's small GPU, the scans (BE's `nearest`,
  `has` from counts) cost more than climbing: few cores, and the levels' serial steps. `has` by
  climbing stays the better form there. On the M3 Max the scans win for BE.
- **x86 needs nothing special.** The sweep, the bulk axes and the spans run at about Apple's speed
  on Zen 3/4 and Sapphire Rapids. Sapphire Rapids' point reads on BE are slower (`findclose`, as in
  t38).

### Step 7: same structure, same bits (E15)

Requirement 3 of 0156 says a repeated subtree keeps the same bits, without hurting decoding.
The spike's fixed record meets it (4b). XF does not yet. Its slots are in PFOR blocks, so each
block picks its own width. And an escaped slot names its exception by its index in the block.
So the same subtree in two blocks can be stored as different bits.

`tape16 repeat <capture>` (`src/repeat.rs`) stores each file's codes and slots five ways:

- **V0**, XF now: codes at the file's width, slots by PFOR blocks;
- **V1**, one width a file, for codes and slots;
- **V2**, one width a language, for codes and slots;
- **V3**, the language's widths rounded up to bytes;
- **V4**, one width a language, chosen to escape at most 2% of the slots.

V1, V2 and V4 have no per-block widths. Their escape is the all-ones slot, which names nothing. Its
value sits in the block's short list of escaped offsets, found by a binary search (8 probes at
most). So equal values are always equal slots.

A subtree counts as repeated if it has 8 nodes or more and its codes and sizes equal an earlier
subtree's, in the same file or an earlier one. The test compares each node's stored fields:
width, slot bits, and the escaped value. A subtree's last slot names the node after it, so that
slot is left out. Positions are XF's (frame of reference and length), the same in every variant.

Results on the quiet M3, one thread (sizes are the same on every machine):

| | data-layer Python | data-layer JSON | rerail Rust | rerail TypeScript | rerail `.rr` | rerail JSON |
|---|---|---|---|---|---|---|
| nodes in repeats, within a file / across files | 20% / 43% | 7% / 79% | 17% / 31% | 19% / 30% | 13% / 47% | 56% / 17% |
| V0: repeats as the same bits, within / across | 63% / 50% | 16% / 22% | 81% / 74% | 89% / 64% | 23% / 22% | 45% / 48% |
| V1 | 100% / 45% | 100% / 18% | 100% / 51% | 100% / 41% | 100% / 52% | 100% / 43% |
| V2, V3, V4 | 100% / 100% | 100% / 100% | 100% / 100% | 100% / 100% | 100% / 100% | 100% / 100% |
| codes + slots in memory, B a node: V0 / V2 / V4 / V3 | 2.12 / 2.17 / 2.34 / 3.13 | 1.58 / 1.82 / 2.34 / 2.22 | 2.14 / 2.21 / 2.44 / 3.14 | 2.15 / 2.22 / 2.44 / 3.14 | 1.89 / 1.93 / 2.60 / 2.21 | 1.72 / 2.42 / 3.03 / 3.24 |
| slots escaped, V2 | 9.3% | 13.7% | 6.4% | 6.7% | 9.5% | 18.7% |
| slot read at random, ns: V0 / V2 / V4 | 2.2 / 1.9 / 1.3 | 3.0 / 2.6 / 2.1 | 2.3 / 1.8 / 1.3 | 2.2 / 1.5 / 1.1 | 2.8 / 2.0 / 1.9 | 2.4 / 6.1 / 1.6 |
| codes + slots on disk, zstd with a dictionary, B a node: V0 / V2 / V3 | 1.15 / 1.10 / 0.75 | 0.41 / 0.37 / 0.34 | 1.32 / 1.32 / 0.91 | 1.23 / 1.23 / 0.88 | 0.84 / 0.71 / 0.56 | 0.64 / 0.60 / 0.42 |
| positions on disk, zstd with a dictionary: as stored / relative (a ceiling) | 2.30 / 1.37 | 1.85 / 0.63 | 2.19 / 1.49 | 2.27 / 1.49 | 2.13 / 1.09 | 1.64 / 0.39 |

The "relative" positions store each start's gap to the previous start, and each length. They
cannot be read in place, so they are only a ceiling for what relative positions could give on
disk. The dictionary is 112 KB, trained on the even files and measured on the odd ones. The load
average on the M5 was 60–100, so the reads were rerun on the M3 at load 3. The M5's sizes are the
same.

- **XF as it is fails requirement 3.** Within a file, only 16–89% of the repeated nodes keep their
  bits; across files, 22–74%. The cause is the per-block widths and the escape indices, as 4b
  expected.
- **One width a language meets it everywhere, for 2–3.5% more memory on code.** On Python, Rust,
  TypeScript and `.rr`, V2 costs 0.04–0.08 B a node more than V0 (2.12 → 2.17 on data-layer
  Python), and its reads are as fast or faster (1.5–2.0 ns against 2.2–2.8), since there is no
  block header to read. The width is fixed per language, like the code numbering, so it is known
  before a file starts and a writer can stream (t35 items 1–5). Codes at the language's width are
  what makes repeats across files the same bits (t35 items 6–9).
- **JSON is the exception.** Its leaf slots store the back distance to the next node's parent, and
  in a long flat array that distance grows with the position in the array. So 14–19% of JSON's
  slots escape at the cheapest width, and 87–98% of those are leaf slots (50–65% on code; counted
  with `E15_ESC=1`). In rerail's JSON this costs +0.71 B a node, and random reads
  take 2.5× as long (6.1 ns), because the escaped values sit in a separate list. Holding escapes
  under 2% (V4) fixes the reads but costs more bytes everywhere. The lever is XF's leaf slot rather
  than the width: parent is asked fewer than 0.01 times a node (E7), so a leaf could store a
  sibling-relative distance, which stays small in a flat array. That is the next thing to price.
- **The same bits up to a shift do not help zstd.** zstd sees bytes, and bit-packed repeats sit at
  different bit offsets. V2 is within 0.05 B a node of V0 on disk; `.rr` gains 0.13. Rounding
  to bytes (V3) makes codes and slots 28–35% smaller on disk (18% on data-layer JSON), but costs
  about 1 B a node more in memory on Python, Rust and TypeScript (+47%). In-memory bytes are 0156's main measure, and the tape is never repacked when it is
  loaded, so V3 is out.
- **On disk, positions are the large term.** With a dictionary they are 2.1–2.3 B a node on code,
  against 0.7–1.3 for codes and slots. A relative form would save 0.7–1.0 B a node on code and
  1.2 on JSON, more than any change to the structure. But absolute positions are what make one
  span two reads in place. The in-place relative forms T9 lists are a block base with offsets
  relative to the subtree's start, an offset from the parent's start, or a start every K nodes.
  They are the next tradeoff to price, alongside JSON's leaf slot.

### Step 8: a leaf slot by sibling, and positions by gap (E16)

Step 7 left two things to price: JSON's leaf slot, and positions in a relative form that is still
read in place. `tape16 relative <capture>` (`src/relative.rs`) prices both at one width a
language, with escapes that name nothing.

**The leaf slot by sibling.** The node after a leaf always has a previous sibling, with the same
parent. So the leaf slot can hold that sibling's size, which stays small in a flat array, instead
of the distance back to the parent. A parent read then steps back from sibling to sibling until a
first child, whose parent is the node before it. Every K-th sibling holds the parent distance
itself, so a read takes at most K − 1 steps. The tags: an inner node's slot is `size << 1`; a
leaf's is `size(sibling) << 2 | 1`, or `parent distance << 2 | 3`.

**Positions by gap.** Each start is stored as its gap to the previous start (in preorder, starts
never decrease), and each end as a length. The absolute start of every K-th node sits in a side
array. A start is the checkpoint plus the sum of at most K − 1 gaps. The sum is read in place a
word at a time: each word's fields are folded pairwise into their sum, as popcount folds bits, and
an exact all-ones test finds escapes, which are rare. The code is compiled once per width, as
the sweep's is. In order, a start is a running sum. Gaps and lengths are the same wherever the
text is the same, so repeated text keeps its bits; the checkpoints sit apart.

Results on the quiet M3, one thread. Every answer is right on every corpus, for both forms and
every K.

| | data-layer Python | data-layer JSON | rerail Rust | rerail TypeScript | rerail `.rr` | rerail JSON |
|---|---|---|---|---|---|---|
| slots, B a node: PFOR (XF now) / XF at one width / sibling K = 8 | 0.99 / 1.04 / 0.99 | 1.03 / 1.19 / 0.92 | 1.01 / 1.08 / 1.03 | 1.02 / 1.09 / 1.06 | 1.02 / 1.05 / 0.92 | 1.20 / 1.80 / 1.12 |
| parent at random, ns: XF at one width / sibling K = 4 / K = 8 | 4.3 / 8.6 / 12.7 | 5.1 / 9.7 / 8.2 | 3.9 / 8.5 / 10.8 | 3.6 / 7.4 / 9.9 | 5.6 / 10.6 / 13.9 | 8.0 / 21.1 / 29.8 |
| positions, B a node: FOR + length (now) / gap K = 8 / K = 16 | 2.48 / 2.07 / 1.94 | 2.41 / 2.14 / 1.99 | 2.40 / 1.97 / 1.85 | 2.42 / 1.96 / 1.84 | 2.35 / 1.97 / 1.85 | 2.48 / 2.25 / 2.09 |
| positions on disk, zstd with a dictionary: now / gap K = 16 | 2.30 / 1.56 | 1.85 / 0.83 | 2.19 / 1.52 | 2.27 / 1.50 | 2.13 / 1.26 | 1.64 / 0.78 |
| span at random, ns: now / gap K = 8 / K = 16 | 4.2 / 8.0 / 12.0 | 4.3 / 17.1 / 25.3 | 3.5 / 8.7 / 12.4 | 3.7 / 8.3 / 11.3 | 3.8 / 10.8 / 16.9 | 4.4 / 8.7 / 12.8 |
| span, every node in order, ns: now / gap | 3.3 / 2.6 | 2.7 / 3.8 | 3.1 / 2.5 | 3.1 / 2.6 | 2.9 / 2.5 | 2.9 / 2.1 |
| repeats whose positions are the same bits, within / across files (gap) | 24% / 5% | 4% / 3% | 31% / 12% | 28% / 8% | 26% / 3% | 12% / 13% |

- **The sibling slot makes one width a language nearly free.** At K = 8 its slots are within
  0.04 B a node of XF's PFOR blocks on code (0.92–1.06 against 0.99–1.02), and smaller on JSON
  (0.92 against 1.03, and 1.12 against 1.20). So step 7's 2–3.5% shrinks to under 0.05 B a node,
  and JSON's +0.71 goes. Every repeat keeps its bits, as in step 7.
- **Its price is the point parent: 2.5–3× XF's on code at K = 8, 1.6× on data-layer JSON and 3.7×
  on rerail's.** A read takes 0.9–1.0
  sibling steps on average on code, and 1.8 on rerail's JSON. Each step is a dependent read and an
  unpredictable branch. Rules ask for a parent fewer than 0.01 times a node, the heaviest rule
  1.3 times (E7). Bulk `has` by climbing reads parents too, so it slows by about as much. `has`
  from counts (step 5) and the axes driven by the S-nodes read no parent.
- **Gap positions cost 16–24% less memory on code** (16–19% at K = 8, 21–24% at K = 16; Python
  2.48 → 1.94 B a node), and 10–17% less on JSON. On disk they are a third smaller or more on code
  and half the size on JSON. Read in order, they are 15–20% faster on code.
- **Their price is the point span: 2–3× today's at K = 8, 3–4.5× at K = 16** (8–11 ns against
  3.5–4.2 on code at K = 8). A point read takes a checkpoint, one or two words of gaps, and a
  length. The fold is 16 ALU instructions a word. The rest is the lab profile's bounds and
  overflow checks, which today's form pays too (177 instructions a read at K = 8 against 76).
  Data-layer JSON is the outlier, at 4×; why is not yet measured.
- **Positions keep their bits only where the text repeats.** Of the structurally repeated nodes,
  24–31% have the same gaps and lengths within a file on code, and 3–12% across files. That is
  the ceiling for any relative form: the rest differ in their text (names, literals, spacing). The
  current form keeps none.
- **Together, on data-layer Python:** codes, slots and positions take 4.60 B a node in XF now. At
  one width a language, with the sibling slot (K = 8) and gaps (K = 16), they take 4.06, 12% less,
  and every structural repeat keeps its bits. The cost is point reads: parent about 3×, span 3×. In bulk
  the new form is as fast or faster.
- **Whether that trade is right depends on how often rules read a span or a parent at a point,
  rather than in bulk.** The bulk-form rule mix is the costing step 1 named. K = 8 is the middle
  choice for both forms.
- **Measurement notes.** The M5's load was 60–100 again, so every timing is from the M3 (load 3–8).
  Getting the gap read right took three passes: replacing runtime divisions by K's shift, an
  any-escape test in place of a popcount, and code compiled once per width. Each is what generated
  code would do. The lab's dev profile keeps overflow and unsafe-precondition checks, for every
  form alike.

### Step 9: how today's engine reads spans and parents (E17)

Step 8 left one question: how often rules read a span or a parent at a point rather than in order.
E17 counts it in today's engine. A build with `--features tape-reads` counts every read of a span
or a parent, from the tape (`transducer_span`, `transducer_parent`, `TapeNode`) and from the facts
arrays built from it (`start`, `end`, `parent`, `subtree_end` in `facts/core.rs`). Each read is
classed by its distance from the previous read of the same part: the same node, the next one,
near (within 16 nodes), or far. Reads made while the facts arrays are built are not counted: in
format 16 those arrays would not exist, and every count here would be a read of the tape itself.

Corpus: data-layer's `impl/sqlite` (221 files, 477,765 nodes). Each of the 168 candidate rules ran
alone, cold for its own results, on tapes already stored. 160 gave counts; the other eight fail
under `RERAIL_SQL_STRICT` or need inputs this corpus lacks.

Reads per node, over every rule:

| | same | next | near | far |
|---|---|---|---|---|
| tape span | 0.337 | 0.203 | 0.701 | 0.332 |
| tape parent | 0 | 0 | 0.0006 | 0.0017 |
| facts start | 0.307 | 1.786 | 1.023 | 0.289 |
| facts end | 0.085 | 1.721 | 0.992 | 0.287 |
| facts parent | 1.186 | 4.490 | 1.467 | 0.358 |
| facts subtree_end | 0.044 | 0.001 | 0.001 | 0.001 |

Point reads (near and far) per node, per rule: spans median 0.47, 90th percentile 4.9, heaviest
17.6; parents median 0.58, 90th percentile 2.7, heaviest 13.7. E7's "parent under 0.01 a node"
counted only the tape's parent; rules read the parent mostly through the facts arrays.

To weigh that, eight rules were timed (CPU, user + sys) with the plain build, from the lightest
to the heaviest by point reads. The extra cost is the point reads times step 8's Python deltas on
the M3: a span +3.8 ns at K = 8 and +7.8 at K = 16, a parent +8.4 ns (sibling, K = 8). The last
column also prices every "next" read as a point read, the case where the engine reads in order
through a point API instead of a cursor.

| rule | CPU, s | point spans, M | point parents, M | extra, K = 8 | K = 16 | next read as points, K = 8 |
|---|---|---|---|---|---|---|
| plan-scope-side-effects | 1.66 | 0.22 | 0.70 | 7 ms (0.4%) | 8 ms (0.5%) | 27 ms (1.6%) |
| grammar-allowlist-burndown | 0.91 | 1.19 | 0.28 | 7 ms (0.8%) | 12 ms (1.3%) | 22 ms (2.4%) |
| serverplan-dispatch-architecture | 1.43 | 2.34 | 0.69 | 15 ms (1.0%) | 24 ms (1.7%) | 35 ms (2.5%) |
| eager-resolve | 3.92 | 3.75 | 2.76 | 37 ms (1.0%) | 52 ms (1.3%) | 70 ms (1.8%) |
| schema-bootstrap-contracts | 1.69 | 5.62 | 0.93 | 29 ms (1.7%) | 52 ms (3.1%) | 73 ms (4.3%) |
| matching-architecture | 4.71 | 6.30 | 5.28 | 68 ms (1.4%) | 93 ms (2.0%) | 116 ms (2.5%) |
| bootstrap-engine-invariants | 5.27 | 7.56 | 5.47 | 75 ms (1.4%) | 105 ms (2.0%) | 125 ms (2.4%) |
| serverplan-quality | 14.18 | 8.40 | 6.56 | 87 ms (0.6%) | 121 ms (0.9%) | 215 ms (1.5%) |

(plan-scope-side-effects ran first and parsed the tapes; its CPU includes that.)

- **The new forms' point cost is 0.4–1.7% of a rule's CPU at K = 8**, and 0.5–3.1% at K = 16.
  Read in order through a cursor, gap positions are faster than today's (step 8), so most rules
  lose less than that.
- **The engine must read in order through a cursor.** Rules read the next node's span 2 times a
  node and its parent 4.5 times; read as points, those would double the cost (1.5–4.3%). A cursor
  over gaps is a running sum, and one over sibling slots keeps the parent of the run.
- **Repeated reads of the same node were mostly one call site, now fixed.** The heaviest rule read
  the same parent 21.9 times a node. With `E17_SITES=1` the count also names each read's caller
  (`#[track_caller]` through the column, `Facts::parent` and `Projected::parent`): 9.2M of the
  10.6M were the syntax table's parent column (`Viewed::value_at`), which read the parent twice
  for one cell. It reads it once now; the heaviest rule's same-parent reads fall to 1.4M, and its
  findings are unchanged. Repeats cost nothing in today's arrays, but each would be a point read
  of format 16 unless hoisted.
- **The rest of the "next" reads are the SQL tables' row-at-a-time columns** (`value_at`, and
  `syntax.rs`'s start/end columns) and one decoded copy: `within.rs` copies the batch's parents
  into an array (5.7M reads in order). On format 16 that copy is a cursor over the tape.
- **Open: on a cold store every file is built twice.** Phase 1 (`sources.rr`, `phases.rs`) runs
  as a nested run that builds each document (tape restored for the matchers, facts decoded), and
  the outer run then builds each again (444 restores for 222 files, 955,731 facts nodes for
  477,765). With the tapes already stored, each file is restored once (221). Not traced further here: it is the
  phase machinery's, and format 16 removes both decodings anyway.
- **The choice: sibling slots and gap positions, both at K = 8.** K = 16 saves 0.13 B a node more
  on positions (Python 2.07 → 1.94) and costs up to twice the point time. Together, at K = 8, data-layer
  Python's codes, slots and positions take about 4.19 B a node against XF's 4.60, and every
  structural repeat keeps its bits.
- **Measurement notes.** The ns deltas are the quiet M3's. The rule times are the M5 Max's, at
  load 39–53. CPU time runs long under load, which makes the percentages read low; the M5's
  per-read deltas are smaller than the M3's, which makes them read high. Read counts do not depend on the machine. A full
  pack run (`run-policies`) fails under strict mode on its first stratum ("a native fact operator
  over input relations"), so each rule ran alone.

### Step 10: what the same bits buy on disk, under zstd and LZ4 (E18)

Same bits only help a compressor that finds them. zstd and LZ4 both replace a byte run seen
before by a copy of it, so a repeated subtree must also be the same *bytes*. A field of w bits
starts at bit `i * w`; at a width that is not a multiple of 8, two copies of a run line up as bytes
only when they start at the same bit phase. `tape16 disk <capture>` (`src/disk.rs`) prices that.
Four forms, each kept as the arrays it reads in place, one stream an array:

- **F15**: format 15's image (explicit columns at byte widths, its own code table).
- **XF**: codes at the file's width, slots in per-block PFOR, starts by FOR, lengths.
- **new, bit-packed**: codes and sibling slots (K = 8) at one width a language, starts by gap with
  a start every 8, lengths; escapes that name nothing (step 8's form).
- **new, byte-aligned**: the same, every width rounded up to a multiple of 8.

Each stream is compressed alone, file by file, with zstd level 3 and with LZ4 (the reference
library, the block format GPU decoders read) fast and HC level 9. Each is run with and without a
64 KiB dictionary, trained per stream on the even files and measured on the odd ones. A second run
compresses the measured files end to end: zstd with a 128 MiB window and long-distance matching,
and LZ4 HC, whose window is 64 KiB. Every stream is decoded and checked equal (none wrong).
Match coverage is the share of input bytes the compressor's matches produce (zstd's sequences,
LZ4's parsed off its blocks).

Bytes a node (measured files):

| | memory | zstd 3 | zstd 3, dict | zstd long, end to end | lz4 | lz4 HC, dict |
|---|---|---|---|---|---|---|
| **data-layer Python** (656 files, 1.32M nodes) | | | | | | |
| F15 | 8.63 | 2.48 | 2.34 | 2.23 | 3.76 | 2.58 |
| XF | 4.59 | 3.70 | 3.37 | 3.17 | 3.98 | 3.24 |
| new, bit-packed | 4.18 | 3.04 | 2.68 | 2.48 | 3.39 | 2.71 |
| new, byte-aligned | 5.55 | 2.15 | 1.94 | 1.82 | 2.85 | 2.05 |
| **data-layer JSON** (730 files, 3.15M nodes) | | | | | | |
| F15 | 9.21 | 1.26 | 1.23 | 0.99 | 1.86 | 1.36 |
| XF | 4.00 | 2.52 | 2.24 | 2.07 | 2.80 | 2.24 |
| new, bit-packed | 3.76 | 1.45 | 1.21 | 1.09 | 1.71 | 1.20 |
| new, byte-aligned | 4.54 | 1.06 | 0.94 | 0.83 | 1.40 | 0.98 |
| **rerail Rust** (193 files, 0.71M nodes) | | | | | | |
| F15 | 8.57 | 2.60 | 2.52 | 2.44 | 3.96 | 2.80 |
| XF | 4.53 | 3.76 | 3.44 | 3.27 | 4.07 | 3.32 |
| new, bit-packed | 4.11 | 3.11 | 2.81 | 2.66 | 3.52 | 2.82 |
| new, byte-aligned | 5.50 | 2.28 | 2.09 | 2.00 | 3.11 | 2.23 |
| **rerail `.rr`** (198 files, 0.25M nodes) | | | | | | |
| F15 | 8.50 | 2.10 | 1.82 | 1.66 | 3.19 | 1.96 |
| XF | 4.20 | 3.31 | 2.93 | 2.65 | 3.57 | 2.76 |
| new, bit-packed | 3.75 | 2.43 | 1.96 | 1.76 | 2.78 | 1.88 |
| new, byte-aligned | 4.46 | 1.67 | 1.43 | 1.27 | 2.26 | 1.44 |

Rerail TypeScript and JSON follow the same pattern (`tape16 disk` prints every language, LZ4 HC
without a dictionary, and the sizes per stream).

Bytes the matches cover, zstd 3 with a dictionary: XF 34–53%, new bit-packed 41–78%, new
byte-aligned 75–86%, F15 90–95%. F15's share is high because most of its bytes are the zero high
bytes of wide fields, which match trivially; it still ends larger.

- **Same bits alone take a fifth off the disk on code, and nearly half on JSON.** New
  bit-packed against XF, zstd with a dictionary: 3.37 → 2.68 B a node on data-layer Python (−20%),
  3.44 → 2.81 on Rust (−18%), 2.24 → 1.21 on data-layer JSON (−46%). LZ4 HC with a dictionary gains
  about as much (−15 to −46%). Matches cover 7–25 points more of the bytes.
- **Byte alignment buys as much again.** At byte widths the same runs line up as bytes, and
  matches cover 75–86% of them. Python 2.68 → 1.94 (−28%), Rust 2.81 → 2.09, `.rr` 1.96 → 1.43,
  JSON 1.21 → 0.94. The new form byte-aligned is the smallest form measured, 17% under F15 on
  Python and 24% under it on JSON. The price is memory: 4.18 → 5.55 B a node on Python, most of it
  codes at 16 bits instead of 9.
- **A mixed form keeps most of both.** Each stream is compressed alone, so the per-stream sizes
  add: codes bit-packed, slots, gaps and lengths at byte widths. That costs 4.63 B a node in memory
  on Python (+0.45 over bit-packed), 2.04 on disk (+0.10 over byte-aligned); Rust 4.59 and 2.24;
  JSON 4.11 and 0.92. Codes compress well at either width (0.43 against 0.30 B a node); the narrow
  fields gain most from alignment (slots 0.57 → 0.35, gaps 0.53 → 0.38, lengths 0.89 → 0.62).
- **The checkpoints do not compress.** They are absolute starts: 0.25–0.32 B a node on disk at
  K = 8, an eighth of the mixed form's total. K = 16 halves them.
- **A 64 KiB dictionary takes most of what crosses files.** Compressing every measured file end to
  end, with a 128 MiB window, saves only 3–12% more than a dictionary does (Python 1.94 → 1.82).
  For LZ4, whose window is 64 KiB, end to end and dictionary are the same within 5%.
- **LZ4 HC with a dictionary is within 1–7% of zstd on the aligned form on code** (13% on
  rerail's JSON), and plain LZ4 is 36–58% larger. LZ4 decodes at 0.2–0.9 ns a node without a
  dictionary and 0.5–3 with one; zstd at 1.3–6 ns. These timings are from the M5 Max at load 38–50, so they are rough. The sizes are exact.
- **So the case for the new form on disk:** against XF, same bits and alignment together take
  35–59% off (Python 3.37 → 2.04 mixed). Against format 15 the mixed form is 11–25% smaller on
  disk (equal on rerail's JSON) and about half the memory.

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
7. **Same structure, same bits (E15).** Done: one width a language, with escapes that name
   nothing, keeps every repeat's bits for 2–3.5% more memory on code. Next: a leaf slot that stays
   small in JSON's flat arrays, and positions in a relative form that is still read in place.
8. **The leaf slot by sibling, and positions by gap (E16).** Done: the sibling slot (K = 8) makes
   one width a language nearly free in memory, JSON included, at 1.6–3.7× a point parent's cost.
   Gaps with a start every 8 or 16 take 16–24% less memory on code and a third less on disk, at
   2–4.5× a point span's cost; read in order they are faster. Next: weigh both point costs by the
   bulk-form rule mix, and write the choice into 0156.
9. **How today's engine reads spans and parents (E17).** Done: at today's mix the new forms cost
   0.4–1.7% of a rule's CPU at K = 8, if the engine reads in order through cursors. Chosen: sibling
   slots and gap positions at K = 8. The repeated parent reads were one call site, fixed. Open: on a
   cold store, phase 1 and the outer run each build every document.
10. **What the same bits buy on disk (E18).** Done: same bits take 18–46% off XF's zstd size, and
    byte widths as much again; a mixed form (codes bit-packed, the rest at byte widths) costs 4.3–4.6
    B a node in memory and 1.5–2.2 on disk on code. Next: write the widths into 0156.
