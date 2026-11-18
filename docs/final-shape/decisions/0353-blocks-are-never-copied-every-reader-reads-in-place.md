# 0353: Blocks are never copied: every reader reads the data where it lives

Status: decided (2026-11-17), by the user: "the entire point is that the CPU and GPU kernels all operate on exactly
the same linear data"; "it must be clear EVERYWHERE that blocks NEVER get copied into different data-structures for
'convenience'". The plan below is being built. Supersedes the parts of 0289 §1–2, 0293 §6 and 0325 step 5 named in
"What it replaces". Follows 0140 (the tape is the engine's bytes), 0266 (blocks), 0293 (one tape API, read in place),
0310 (reads written once, three backends) and 0325 (relations are blocks).

## Intro

**The problem.** The engine holds its data in blocks: each document's tape, and every relation the rules derive
(0325). The CPU and Apple's GPU share one memory, so both can read a block where it lies. They do not. Between the
tape and the kernels that join over it there are three copies, and more after them:

1. A document's tape is numbered into the projection's per-node arrays (`Projected`: `core`, `local`, `up`, `idx`,
   `fielded`, `size`, `depth`, and the definition ids' `low`, `order` and `back`).
2. For the device, each node's start, end, kind and field are read off the tape again and written into the chunk
   block (`device_chunk::build`), beside the projection's arrays. That is about 38 bytes a node in all.
3. A stratum on the device then expands each view it reads into a relation of its own (`expand_view`): rows ×
   columns × 8 bytes, made again for each stratum that misses the residency cache.

Measured on PostHog's `posthog/hogql` (437 files, 1.3 M nodes, 2026-11-17, the light run on Metal):

| Copy | Size or cost per run |
|---|---|
| Views the device expands into relations | 660.8 MB, 111.5 ms of host CPU |
| Views and names still uploaded from the host | 7.4 MB, 22.1 ms |
| `core.rr`'s per-node rows, made on the CPU (`node rows`) | 0.45–0.49 s of CPU, about 10% of the run |
| The chunk block's node columns (`a batch's chunk block`) | 0.07 s of CPU |

These copies are also why step 6 of 0325 measured slower. The device's memory budget filled with view and index
copies. Strata were then refused at run time and redone on the CPU, and strata ran apart from their wave 103–181
times a run against 36–55 (0325's Built entry, Q68).

**What changes.** One rule, with no exceptions for convenience: **data lives in one place, in one layout, and every
reader reads it there.** That means the CPU executor, the JIT's code, the Metal kernels and SQLite's virtual tables.
A view such as `node` or `child` is not a relation that anything makes. It is a read of the tape. A kernel joining
over `child` reads the tape's children where the tape lies, on the CPU and on the GPU alike.

**Before and after**, for a stage that joins `child(p, i, c)` on the GPU:

```text
before:  tape --walk--> Projected arrays --copy--> chunk block --expand--> child relation --read--> stage kernel
after:   tape --------------------------------------------------------------------- read --> stage kernel
```

**The prior work agrees.** In the ast-index-spike, 95% of the cost of sharing work between queries was copying into
private tuplestores, not decoding (design index §5, D S149, R L1176-1223). Reading the tape through its address, not
its value, took a read from O(file) to a flat 0.13 µs (§"Address, not value", D S063). And per-row bookkeeping, not
decoding, dominated (D S152).

**Status.** The rule is in force from this record: `AGENTS.md` states it, and `scripts/check/check-no-block-copies.py`
refuses a commit that adds a copy. The copies that exist today are listed below, each with the step that removes it.
Nothing is measured as this record's effect until the steps are built.

**How to read this record.** "The rule" says what a copy is and what is not. "Where data lives" and "How readers
read" are the shape. "Node identity" settles the one hard question: a node's id is not on the tape. "The copies
today" is the inventory, and "The plan" removes it step by step.

## The rule

1. **A block is never copied into another data structure.** Not into columns, rows, records, per-node arrays, a
   device relation, a staging vector or a buffer of the device's own. Not for a reader's convenience, a faster
   loop, a layout a kernel prefers or a missing accessor. A missing accessor is written, once, as a read (0310).
2. **A copy** is any value read from a block or tape and written somewhere else for a reader to read instead. Moving
   the tape into a struct by `Arc`, or binding a block's buffer, copies nothing.
3. **Not copies**, and so allowed:
   - **A load.** Bytes come from disk into memory as the exact bytes the readers read: a stored tape or block
     inflated from zstd, or a 16z block decoded to the tape's own bytes for that block (0293 §2). A load never
     changes the layout.
   - **A derived fact, written once where it is computed.** A rule's head row, an axis's answer, a text interned to
     its symbol, a parsed integer. These are values no read of the tape gives by itself. Each is written once, by
     the step that computes it, into a relation in a block, and read there by every reader. A value a read of the
     tape gives by itself (a node's kind, span, parent, children, field, line, depth) is never stored as a fact. It
     is read.
   - **An index**, in the block of the relation it indexes (0325 step 5). It is built from the relation, not from a
     copy of it.
4. **Named exceptions, each removed by a step below:** the copies listed in "The copies today". Every one carries a
   comment naming this record and its step (`copy (0353 step N)`). The list only shrinks.
5. **A block that outgrows its space** moves once to a larger one (`facts::rows`'s tail, as a vector grows). It is
   named and counted in the profile. Blocks reserve address space, not memory, so that this does not happen (as the
   chunk block does). It is not a way to change a layout.

## Where data lives

| Data | Its one home | Read by |
|---|---|---|
| A document's tape (format 15, 16 or 16z), its source text, its line starts, its language's code table | The chunk block, placed as stored (0293 §9) | Every reader, through the tape reads (`tape_*`, 0310) |
| A document's granule table: each granule's slot, first and last tape node, and count (see "Node identity") | Beside its tape in the chunk block, written once when the document is placed | The id reads (`node_id`, `node_of_id`) |
| A relation a rule derives, its dead rows and its indexes | Its block (0325) | The relation reads (`rows_*`, `rel_*`) |
| An axis's answer (`rr_nearest_*`, `rr_inside_*`, `rr_has_*`) | A relation in the chunk block, written once by the solve on the CPU or the GPU | The relation reads |
| A text's symbol, and the symbol table's strings | The run's strings block (`atoms`) | Both sides, in place |
| The output's, frames' and memo's rows | Relations in blocks (0325 step 7) | Read where they lie; no copy into output rows |

## How readers read

**A stage reads one of two kinds of input:**

- a **relation**, through its descriptor: the block's base and the relation's offset;
- a **tape view**, through its descriptor: the chunk block's base, the document table's offset, and which view it is.

The descriptor is the same bytes on the CPU and on the GPU. The reads are the generated ones (0310): Rust for the
host executor and SQLite's vtab, CLIF for the JIT, which inlines the hot ones, and MSL for the kernels. No reader has
a read of its own.

**A view is a read, not a relation.** Each base view has a scan form, which enumerates its rows, and a probe form,
which finds the rows for a key. Both read the tape where it lies:

| View | Scan | Probe by | Probe reads |
|---|---|---|---|
| `node(d, n, k, s, e)` | Each projected node, 64 at a time by the code table's mask (0293 §1 "in bulk") | `n` | id to tape node; then code, start, end |
| `child(p, i, c)` | Each node with its parent, carried in the walk (the subtrees open around it, 0293 §8) | `p` | First child, next sibling hops |
| `field(p, f, c)` | As `child`, with each child's field code | `p`, `(p, f)` | As `child`, testing the field code |
| `core_node`, `node_order`, `tape`, `ast_depth` | One walk: tape position, preorder, subtree end, depth carried | `n` | id to tape node; subtree end; depth by a climb |
| `line`, `end_line`, `col`, `docof` | Each node's start or end against the document's line starts | `n` | Span read, then a search in the line starts |

On the GPU, a scan is a dispatch over groups of 64 nodes (one thread a group, as 0293 §10's roots), and each group
finds its open subtrees by one descent from the root (`Tape::path`). A probe is a read from the key. Nothing is
materialized in between.

**Values the tape does not hold** are derived facts. A text's symbol (`text`, `parts`, `words`), an integer's value
(`intval`) and an operator's code (`op_t`) are each computed where a rule reads them. The step that computes them
writes them once into a relation in a block, and the readers read it there. Today `core.rr`'s node rows compute
them eagerly for every visited node of every chunk. They become rules the planner runs on demand.

**SQLite's virtual tables** read relations and tape views through the same reads (0325 steps 1 and 2 for relations).

## Node identity

Relations hold a node as its id by definition (0083 §3, 0094 §2): `document << 32 | slot << 18 | index`. A granule's
slot is a hash of its key, with clashes settled in key order. Its index counts the granule's own nodes in preorder,
then its tokens. Both are functions of the tape (`granules.rs`). An id is not a number on the tape, so until step 2 a
per-node array held it (`DefIds::low`, 4 bytes a node).

**In place:** a document keeps a granule table (`granules::Table`), written once where its granules are numbered.
It holds, per granule:
- its start and its subtree's end on the tape, and its parent granule;
- its slots;
- its numbered nodes' and tokens' counts, as prefix sums;
- its child granules, each with the granule's own nodes and tokens before it.

Beside it is a rank index: per 64 tape nodes, which ones are numbered nodes and which are tokens, and how many of each
come before. That is 24 bytes per 64 nodes, 3 bits a node. The array it replaces holds 32 bits a node (`DefIds::low`),
plus 64 for its way back.

- **The id of a tape node:**
  - its granule is the innermost one whose subtree holds it (a bisection of the starts);
  - its index is its rank among the granule's numbered nodes, read off the rank index (two ranks), less the counts of
    the granules nested between;
  - a token's index follows the granule's nodes.
- **The tape node of an id:**
  - the slot's granule, then the child granule its index falls after (a bisection);
  - then a select forward from that stretch's start.
- **A scan** walks the rank index's set bits and carries the innermost granule and its running counts.

A granule's tokens are numbered in preorder of the tokens, not in the order of their nodes. That is what a rank
counts. It moved the pinned ids' hash once (`definition_ids_are_pinned`; nodes' and spans' ids are unchanged), and
`Locs`' token spans follow the same order.

**Why a rank index, from the measurement** (`native/examples/node_ids_in_place.rs`, hogql: 437 files, 1,311,480
ids, 20,877 granules; least of 5 rounds; load 40–55):

| Read | Instructions an id | Cycles an id |
|---|---|---|
| The array (`DefIds::low[l]`) | 4.6 | 1.2 |
| Point read, counting the codes from the granule's start | 6,260 | 820 |
| Point read, rank index | 171 | 26 |
| Scan, rank index | 59 | 7.8 |
| Node of an id: the array's way back (a sorted copy, made once) | 209 | 45 |
| Node of an id: rank index, bisection, select forward | 254 | 74 |

Formats 15, 16 and 16z measure the same with the rank index, since it does not read the tape's codes. The projection
that makes the arrays costs 1,350 instructions an id in format 15 (1,770 M), 1,500 in 16 and 2,450 in 16z.

- **The codes alone cannot say which nodes are numbered.** A code says what a node is, not what is above it. Python's
  `is not` is an anonymous node holding the anonymous `is` and `not`: the numbering counts it as a token of its
  named parent, and does not count its children. hogql has 5,348 such nodes.
- **A count of the codes costs about 1,210 instructions per 64 nodes** in formats 15 and 16, and 1,740 in 16z: set
  membership is tested one code byte at a time. A rank directory alone (counts only) would still read two such
  groups a point read.
- **A granule can be large.** A document with more granules than slots folds into one: 17,000 definitions are one
  granule of about 136,000 nodes. Counting from its start is unbounded; a rank is not.

The rank index is derived: which nodes the projection numbers is a function of the codes and the parents, written
once. It is an index, not a copy of the tape. The check (`node_ids_in_place_equal_the_numbered_ones`) runs every
node and token of Python, TypeScript, TSX, JavaScript, Rust, PHP and SQL documents, and of the folded one, in formats
15, 16 and 16z and both shapes:
- the id equals `DefIds::low`'s;
- `Table::node` inverts it;
- the scan gives the same ids;
- no other tape node has one.

Findings stay 1,629 on hogql. Interleaved first runs on fresh stores, at load 22–34: the base retired 80.0 B
instructions, and with the table built beside the arrays 71.8 B, inside the earlier spread of 65.7–80 B. The table's
build is 0.05 s of CPU (the `granule slots` span).

`Projected`'s `core`, `local`, `up` and `size` are gone (2026-11-17). Which tape nodes are the projection's nodes
and tokens is the rank index (`syntax/ranks.rs`, in the batch's block): per 64 tape nodes, the nodes' bits, the
tokens' bits and the counts before, and a select sample per 64 items. The granule table reads it too, so the table
holds no groups of its own. The reads, by kind of reader:
- a pass visits the set bits in order (`Projected::nodes`), and with the open subtrees it carries each node's parent
  (`walk`, `walk_up`), not a parent read a node;
- a node's byte spans in local order come from one pass (`spans`);
- a point read selects once a row (`Viewed::row_at`, however many columns read it);
- the vtab's bulk reads (`filter_eq`, `rows_of`, and the new `Source::for_rows`, which the device's upload uses) visit
  each document's nodes once, and walk only when a column reads the parent.

Checks: the walk's parent equals `parent` for every node of the test's documents, in every format and both shapes, and
a view's row passes equal its per-row reads (`for_rows`, `rows_of` and `filter_eq` on `node`, `child`, `field`,
`tape`). Findings stay 1,629 on hogql. Three interleaved pairs on fresh stores, at load 45–58: 232.4 B instructions
for the base, 227.4 B without the arrays, inside the spread of each binary's own runs (±5 B). The span profile's self
time is 6.34 s against 6.35 s.

The first build without the arrays was 7% more instructions. Sampled call stacks showed where: each loop that went
`0..len` and selected a node a step (`Ranks::select`), a scan that read the tape parent a row (`Facts::parent`), and
the vtab's reads selecting once per column. Those are now the passes above. What still costs, by the span profile: `open: input load` (+0.09 s), where `setfix::view`'s filter reads each
candidate row's values by point reads.

`fielded` is gone too: the `field` view's rows are a third bit set in the rank index (`Item::Field`: the nodes
below the root under a field), five words a group. A scan tests the bit; a row's point read is a select among
them, which brings the tape node with it. Four interleaved pairs at load 26–62: 315.6 B instructions with the
array, 312.8 B without.

`idx` (a node's index among its siblings) and `depth` are not the tape's data: no tape stores them, and a read
in place would climb the parents (a search back on format 15) every time. They are derived values, and go as
step 3 makes the other derived values go: rules computed on demand, written once into relations. Until then they
are computed once a projection, on first read, into its block (row 1 below, now marked step 3).

Step 3, as built for them (2026-11-17):
- **A scan carries both in its walk.** `Viewed::each_row` keeps the open subtrees (each one's end on the tape, its id
  and its children so far), so a row's parent id, its place and its depth come with it. `Projected::places` is the
  same walk for the other passes (`Within`'s statistics, the device chunk's `ord`, a step 4 copy).
- **A point read of `depth`** climbs the tape's parents and counts the numbered ones.
- **A point read of `idx`** reads a derived column. The tape holds no sibling's place, and hops along the siblings
  are as many as the siblings: a probe for each of a module's 17,000 functions was quadratic. So the first point
  read of a projection writes the walk's places once into its block (`Projected::idx`, `copy: derived`). A scan
  never makes it.
- hogql, 4 interleaved pairs at load 112–134: findings 1,629 in each; 78.3 B instructions (median) against 77.0 B,
  within the noise. The derived column is made for 719–783 projections a run (0.03 s, 0.2–0.3 MB); the two arrays
  were made 2,731–2,841 times (0.075–0.081 s, 21–22 MB).
- Check: in `node_ids_in_place_equal_the_numbered_ones`, each node's point reads equal the walk's, in every format,
  whole and narrowed; `a_views_filter_scan_holds_the_rows_a_per_row_read_finds` holds the scans to the point reads.

**Step 3's derived values: `str` on request** (2026-11-17). `str(n, s)` is answered on request, as 0174 answers
`node_text` and `value`: `ON_REQUEST`'s second round, answered by `@rr_node_str(n)` (`LongTexts::str_any`), which
reads the node's literal as the batch's shape states it (`core_rr::node_str`, the definition the eager rows use too,
or T's). Check: `str_answered_on_request_holds_the_eager_rows`, against a reader that keeps it eager. On hogql
(`p0_names` asks for 2 nodes) it saves almost nothing, and the reason is in the profile: the projection's skip
flags compute each node's literal wherever `text` is read (a literal's prefix makes it token-like), and hogql's
rules read every node's `text` (no kind narrows them, `text_nodes` is none), so `CoreRows::node` visits all
1.3 M nodes for `text`. Rules look `text` up by node for 663,057 rows. `text` is what row 7's time is.

`text` on request is open (work queue Q121). A first build (a node's whole text answered by `@rr_text`, the spans'
rows eager as `rr_text_span`) broke the widened copies' termination, the converter's printing and a demand test, and
0174 §4 measured that a request a node costs more than eager `text` within the bound. It is kept as a patch until it
is measured against eager on hogql.

Found on the way and fixed: the image's merge (`preparation::merge`) merged two requests of one body
(`rr_str_req`, `rr_node_text_req`), which the tree's keeps by name (`simplify::request`, the debug check "the
version's merges are the tree's"); `node_column_counts` indexed a text document's dialect, which it has none of.

**Step 3's tape answers: `next_sibling` is a view** (2026-11-17). Its rows were core.rr's `CoreRows::node` pairs,
each numbered node's numbered direct children, consecutive. Now:
- the rank index marks a fourth item, `Item::Sibling`: a node after a numbered direct sibling (the projection's walk
  keeps the numbered nodes on its path and whether each has had a direct child, popped down to each node's tape
  parent, which the walk gives; an anchor that hangs from the root is no child of it). Popped by subtree ends, the
  marking added 0.014–0.032 s to hogql's identity maps (0.087–0.093 s); popped by parents, 0.108–0.116 s against
  0.096–0.113 s at load 70–77, within the noise. A group is six words, 48 bytes for 64 tape nodes (40 before); `Numbering` holds four counts and
  samples, its Rust and MSL reads written again from the CLIF;
- `View::Sibling` has a row per marked node, as `field`'s are its marked ones. Column `b` is the row's node; `a`
  comes with the scan's walk (the open subtree's last child), and a point read climbs from the tape node before `b`
  (`Projected::prev_sibling`); a lookup by `a` hops the tape's siblings after its subtree (`next_sibling`);
- check: `node_ids_in_place_equal_the_numbered_ones` holds both reads to the old pairs in every format, whole and
  narrowed, and the rank reads of the new item to its walk. The device has no view of it yet (step 4).
- hogql, 3 interleaved pairs against step 2's end (`s2s`), load 70–77: findings 1,629 in each; 76.3 B instructions
  (median) against 79.2 B, within the noise (72.9–82.5 B against 74.5–81.8 B).

`line`, `end_line`, `col` and `docof` were views already (`View::Loc`) except where `Extra` makes spans' rows; those
and `visit_index` (CPython's visit order, a derived fact computed once for the documents a program reads it of),
`doc_root` and `host_node` (a guest's) are left for this step.

Found on the way and fixed:
- `Locs::token_spans` copied every token's start and end into a `Vec` on a span's first read: unmarked, and missed by
  the check. A token's span is now its tape node's, by a select on the rank index.
- `front::prune` gated a request rule by the readers lowering recorded (`asked_by`). A reader inlined since
  (`simplify`) is read under its readers' names, so the gate dropped the request: `tables :- q(s, x0), …` with `q`
  inlined read no `str` (`value_documents_are_memoised_with_the_files_that_asked`). A record naming a relation the
  program no longer has now leaves its rule ungated.

`DefIds`' `low`, `order` and `back` are gone. Every projection, narrowed ones too, keeps its granule table, and it
answers ids, order keys and their inverses:
- `order`, an order key (its run's rank, then its index in its granule): the table keeps each granule stretch's run
  rank (`runs`, `run_at`), and falls back to preorder indexes when the runs or a granule's nodes outgrow the key's bits
  (`preorder_keys`);
- the table keeps each granule's start and end as ranks too, by item (`rank_starts`, `rank_ends`), so a point read goes
  from a node's or token's number to its id with no select (`low_of`), and back (`number_of`, `number_of_order`);
- a pass reads the ids with the walk (`granules::ids`; `Projected::node_ids`, `token_ids`, `loc_ids`), carrying the
  open granules and their counts;
- a view's bulk read walks the ids only when a column reads a row's or its parent's id (`Viewed::reads_id`);
- a row's document is its batch document's key (`Source::documents_of`), not its id's high bits read row by row.

`granules::Reference`, the old numbering's `low` and `order`, is made only for the check
(`node_ids_in_place_equal_the_numbered_ones`), which now covers narrowed projections and the passes' ids.

The first build without the arrays was 6–19 B instructions over the base on hogql. Sampled call stacks put it in
point reads where a pass held the node (a select each, then the table), the id walk run for views whose read
columns hold no id, and `owners_delta` reading every column of every row. With those fixed: 12 interleaved pairs at
load 40–90, findings 1,629 in all, median 75.7 B against 81.2 B for the base (79 B without one run of the base
caught by a load spike). What costs, by the samples: the id walk in passes that write a per-node derived row
(`Extra::nodes`, under 1%), and a pair's id read by a point read (`Within::value`).

The measurement found a race the timing change exposed (work queue Q90): a parked stratum resumed on another thread
than its submission's found that thread's stratum of the same per-thread ticket, and ran it with its rules (a panic
in 1 run of 20, 1,422 findings in another). Tickets and waves are now numbered for the process.

The table's columns are in the projection's block beside its rank index (`cells::Cells`), written once where they
are computed. Six interleaved pairs on hogql: findings 1,629; the samples show no cost in the placing, and the
table's build unchanged.

The reads are written once in CLIF (0310), in the relation's family: a read takes the address of the projection's
descriptor (`rerail_clif::ids::Numbering`, in its block), whose runs point at the rank index's groups and samples and
at the table's columns. `num_is`, `num_rank`, `num_select`, `num_low_of`, `num_number_of`, `num_order_of` and
`num_number_of_order` (`rerail_clif::id_reads`). Their Rust is the facts crate's `id_reads.rs` and their MSL its
`id_reads.metal`, each kept current by a test in `rerail-clif`. `Ranks` and `granules::Table` read through the Rust;
their hand-written point reads are gone.

Checks: the reads equal the hand-written ones on every projection of the id test (done before those went); now the
rank index's point reads equal its walk over the set bits, the ids and order keys equal the old numbering's, and on
the device the MSL reads answer what the Rust answers for every tape node, in its block, in all three formats, whole
and narrowed (`the_kernels_id_reads_are_the_hosts`). hogql, 8 interleaved pairs at load 55-91: findings 1,629; median
75.1 B instructions against 78.2 B before.

Step 2 is done but for one copy: the chunk's `low` column (row 3), which goes when step 4's kernels read the ids
through `id_reads.metal`. Each generated MSL file defines the same load helpers (`clif_word`, ...): a kernel that
includes two of them needs those emitted once, which step 4 does as it includes them.

## The copies today

Each row is marked in the code with `copy (0353 step N)`.

| # | Copy | Where | Size or cost (hogql) | Removed by |
|---|---|---|---|---|
| 1 | Gone (step 3): the projection's per-node arrays. `core`, `local`, `up`, `size` and `fielded` are the rank index; scans carry `idx` and `depth` in the walk, and a point read climbs for `depth`. Only a point read of `idx` makes a column, derived once (see "Node identity") | — | — | Done |
| 2 | Gone (step 2): definition ids by node, `DefIds::low`, `order`, `back`. The granule table answers them. | — | — | Done |
| 3 | The chunk block's node columns: `core`, `up`, `fielded`, `low`, `start`, `end`, `kind`, `field`, and the canonical code copy. `low` is made by one scan of the granule table. `core`, `up` and `fielded` are made here from the rank index and the walk, now that the host holds no arrays of them. | `device_chunk.rs`, `build` (`made`, `made_in_order`, `put`) | 22 bytes a node; 0.07 s a run | Step 4 |
| 4 | Views expanded into device relations | `metal_join.rs`, `expand_view`; `device.rs`, `resident` | 660.8 MB, 111.5 ms a run | Step 4 |
| 5 | Views, flat relations and names uploaded from the host | `metal_join.rs`, `DevRel::upload`; `device.rs`, `resident`, `texts` | 7.4 MB, 22.1 ms a run | Steps 4, 6 |
| 6 | An axis's pairs copied into the chunk, then expanded to ids | `within.rs`; `device_chunk.rs`, `put_pairs`; `VIEW_AXIS` | 8 bytes a pair, then the expansion | Step 5 |
| 7 | `core.rr`'s and the vtab's per-node rows: tape answers (`line`, `end_line`, `col`, `docof`, `visit_index`, `doc_root`, `host_node`) and eager derived values (`text`, `parts`, `words`, `intval`, `op_t`) | `syntax.rs` `node rows`, `NodeRows`; `core_rr`; `tree.rs`; `guests.rs` | 0.45–0.49 s of CPU a run | Step 3 |
| 8 | The CPU's head rows, staged in `Fresh`, merged by `extend_from_slice`, then pushed into the relation | `setfix.rs`, `Fresh` | Every row a CPU stratum derives | Step 6 |
| 9 | A second key's index copies the relation (`DevIndex::moved`); a relation over several blocks merged for the device (`in_block`) | `metal_join.rs`; `facts/rows.rs` | Q67 counts them | Step 6 |
| 10 | A device fixpoint's total copied into a run's block at its end | `device.rs` (0325 step 4) | Each recursive stratum's rows | Step 6 |
| 11 | The transducer's packed batch: 44 bytes a node, and its child lists | `PackedBatch::nodes` (0293 "Interim") | Each matched batch | Step 7 |
| 12 | Findings, frames and memo entries copied into rows of their own | The output, frames and memo stores | Not yet measured | Step 8 (0325 step 7) |

## The plan

Each step ends with:
- its copies gone from the code;
- its marks gone;
- the profile's `copies (decision 0353)` counter showing that step's bytes as 0;
- findings unchanged (1,629 on the light hogql run);
- instructions and peak measured against the step before, in interleaved runs, with the load noted.

1. **The rule, everywhere, and the counter.**
   - This record, `AGENTS.md` and the commit guard.
   - Each copy site above marked.
   - One profile heading that counts every copy that remains, by site, in bytes and CPU: the measure the steps drive
     to 0.
   - 0289, 0293 and 0325 point here where this supersedes them.
2. **Node identity in place.**
   - The granule table is placed beside each tape.
   - `node_id` and `node_of_id` are written once as reads (0310: Rust, CLIF, MSL).
   - Checks:
     - every node's id equals `DefIds::low`'s, in formats 15, 16 and 16z, for every dialect with a granule rule;
     - `node_of_id` inverts it.
   - Measured before it replaces anything: a point read and a scan, against the array.
   - Then `Projected`'s arrays and `DefIds`' go. The CPU's `Viewed` reads the tape and the granule table.
3. **The base views read the tape on the CPU.**
   - The executor's input for a view is a tape view.
   - The scans and probes in the table above are generated reads, inlined by the JIT.
   - `core.rr`'s and the vtab's per-node rows of tape answers go: they are tape views.
   - Its derived values (`text`, `intval`, ...) become rules computed on demand, written once into relations.
   - Check: the views' rows equal today's, for every view and dialect, cold and warm.
4. **The base views read the tape on the GPU.**
   - A stage's input may be a tape view: the kernels read the chunk block's placed tapes through the generated MSL
     (`metal_tape.metal`'s reads, 0293 §9).
   - A scan is a dispatch over 64-node groups.
   - `expand_view`, `VIEW_EXPAND`, the chunk block's node columns and the residency cache's view entries go.
   - Check: the device's rows equal the CPU's (`device::parity`), for every view.
   - Measure: the 660.8 MB is 0, and the device budget's refusals and strata run apart, against Q68's.
5. **Axes are derived relations written once.**
   - The solve, on the CPU or the GPU, writes its answers as ids into a relation in the chunk block, which both read.
   - `put_pairs` and `VIEW_AXIS` go.
   - `has`, made per looked-up node, is a probe that climbs the tape, not an upload.
6. **The executors write and index in place.**
   - The CPU's head rows are claimed and written in the relation's block, as the device's are (0325 step 4). A
     round's new rows are the relation's delta range; `Fresh`'s staging and merge go.
   - A relation is made with room for the indexes its readers will probe, which the planner knows when it plans the
     stratum. `DevIndex::moved` goes.
   - A device fixpoint's total is in a block of the run's life from its start.
   - The relations of a run lie in one slab's buffer, so `in_block`'s merged copy goes.
   - The text tests read names in the strings block, and `DevRel::upload` is deleted.
7. **The transducer reads the tape.** The packed batch's node records and child lists go (0293's interim): the CPU
   JIT's roots, the grid and the register machines read the tape. This is the transducer's owners' work. It is
   listed so that the rule holds everywhere.
8. **The output, frames and memo** are relations in blocks, read where they lie (0325 step 7).
9. **Placement is priced again** on what is left: no upload, view or copy term (0325 step 6, Q68).

The order removes the measured cost first: step 4 is the 660 MB. Steps 2 and 3 come before it because the GPU's
reads need the ids, and the CPU's reads are their check.

## What it replaces

- **0289 §1–2.** The chunk block holds the placed tapes, the line starts and the granule tables. It holds no node
  columns, no projection arrays and no view made from them. `VIEW_EXPAND` goes.
- **0325 step 5.** "The views it makes from a chunk … become relations in blocks of their own life" is withdrawn
  for views. A view is never made. The device's own relations that are derived (a fixpoint's total and delta, a
  lattice's total) stay relations in blocks.
- **0293 §6, `Projected::up`.** A per-node copy: it goes with step 2. Scans carry the parent in the walk and point
  reads use the tape's parent, so nothing reads it node by node.

## What it keeps

- 0140 and 0293: the tape is the engine's bytes, read in place, in every format, through one API. No decoding.
- 0266: blocks are big things with everything nested, self-relative. A delta reads with its base.
- 0310: one definition of each read, in three backends. The JIT inlines the hot ones.
- 0325: relations are blocks that the CPU and the GPU read and write in place, and their indexes are theirs.
- 0083 and 0094: node ids by definition, stable across edits. The same ids, computed in place.
- 0104: findings are the CPU binding's. Every step keeps them.

## Checks

- `scripts/check/check-no-block-copies.py` (pre-commit) refuses a commit that adds a copy. It flags any added line
  in the engine's data paths that writes values read from a block or tape somewhere else (`copy_nonoverlapping`,
  `copy_from_slice`, `extend_from_slice`, an upload, an expansion, a row read into a `Vec`), unless the line or the
  one above says which allowed kind it is:
  - `copy: load`, `copy: growth`, `copy: derived` (a fact written once where it is computed);
  - `copy: not block data`;
  - `copy (0353 step N)`, an existing copy being edited.
- The profile's `copies (decision 0353)` heading: every remaining site's bytes and CPU. Each step drives its sites
  to 0.
- Each step's checks above, and the parity tests on both bindings.

## Open

- Texts compared in place (spans' bytes) rather than through symbols: a later question, after step 3.
