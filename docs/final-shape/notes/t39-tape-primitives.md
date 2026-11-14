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

## 4. The plan

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
   - Narrower codes a block: each block gets its own small code table (0.70 B a node today), so
     repeated structure gives repeated bits.
   - MF's gap end with the gap stored beside the next start.
4. **BE.** A two-level range min-max tree for `findclose` and `enclose` (Navarro–Sadakane), and
   Elias–Fano select by sampling.
5. **Metal.** The bulk primitives as kernels: the stack sweep as a scan over each block's depths,
   with carries between blocks.
6. **Rerun everything on the seven machines** (t38 sections 5 and 6).
