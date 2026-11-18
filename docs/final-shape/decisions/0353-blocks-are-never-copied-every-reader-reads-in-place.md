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
is measured against eager on hogql. Deferred (2026-11-17): `text` stays eager within its narrowing until steps 3 and 4 are done; after the literal
fix below its rows visit 0.84 M of hogql's 2.27 M nodes, and rules look it up for 663 K rows.

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

**Step 3: literal rows only where they are read** (2026-11-17). `rr_value_lit` (the eager part of `value` on
request, 0174) holds the literals of the documents W reads no values of. The skip flags came from the program's
reads, so a Python batch computed every node's literal and `values_of` dropped them all, and `CoreRows` visited
every node for them (`Visits::All`). Where no other relation reads a literal, a document whose dialect W values now
makes none, and the walk visits only the nodes whose text a rule reads. hogql, 3 interleaved pairs at load 43–49:
nodes visited 2.24 M to 0.84 M of 2.27 M; `node rows` 0.427–0.433 s to 0.277–0.304 s; atoms interned 0.011–0.012 s to
0.007 s; findings 1,629 in each; instructions within the noise (71.6–82.3 B against 75.0–81.5 B).

**Step 3: places are views where no span is made** (2026-11-17). `line`, `end_line`, `col` and `docof` were views
already (`View::Loc`), but `Extra` made `end_line`, `col` and `docof` as rows for every node and token wherever a
program read them, and `Batch::made` bound those rows in place of the views (`line` alone waited for spans,
`line_rows`). In core.rr's shape they are now made only where `Extra` makes spans, which have places too. hogql reads
no span: 3 interleaved pairs at load 23–25, `extra document rows` 0.094–0.135 s and 254–271 MB allocated to none;
findings 1,629 in each; 73.5 B instructions (median) against 80.2 B.

Where spans are made, the nodes' and tokens' place rows are still made beside the spans' (the spans' are derived
facts; the nodes' are copies of tape answers), and bound in place of the view. The view already covers the whole
source and its lines (`Locs::positions`, the `ExtraSpan::Line` ids); the other spans (`src_line_lf`, `tape_line`,
matches, comments, `rx_span`) have their byte ranges only in `Extra`. Q123: those ranges written once into the
batch's block, and the view reading them past its own positions, so nothing makes a node's place.

**Step 3's other relations of row 7** (2026-11-17), each checked against "Not copies":
- `visit_index` (CPython's visit order), `host_node` (a guest node's innermost host node, an axis's answer) and
  `doc_root` (a document's root id, one row a document) are derived facts, each made only where a program reads it
  (`tree::Tree::new`, `GuestRows::document`'s `wants`). They stay relations.
- `intval` and `op_t` are read by no core.rr library rule; in core.rr's shape `NodeRows` runs only where a program
  reads them. In T's shape they come with `NodeRows`' eager `text`, `name` and `str`, deferred with Q121.
- What row 7 keeps: `core_rr`'s and `NodeRows`' eager derived values (`text`, `name`, `value`'s literals, T's
  rows), deferred (Q121), and the nodes' place rows beside spans (Q123).

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
| 6 | Gone (step 5, 2026-11-18): an axis's pairs copied into the chunk, then expanded to ids. The solve writes ids once into a relation of its own, which every reader reads where it lies; `put_pairs` and `VIEW_AXIS` are deleted. `has` is still made per looked-up node and uploaded (row 5) until its probe climbs the tape | — | — | Done |
| 7 | `core.rr`'s and the vtab's per-node rows: tape answers (`line`, `end_line`, `col`, `docof`, `visit_index`, `doc_root`, `host_node`) and eager derived values (`text`, `parts`, `words`, `intval`, `op_t`) | `syntax.rs` `node rows`, `NodeRows`; `core_rr`; `tree.rs`; `guests.rs` | 0.45–0.49 s of CPU a run | Step 3 |
| 8 | The CPU's head rows, staged in `Fresh`, merged by `extend_from_slice`, then pushed into the relation | `setfix.rs`, `Fresh` | Every row a CPU stratum derives | Step 6 |
| 9 | A second key's index copies the relation (`DevIndex::moved`); a relation over several blocks merged for the device (`in_block`) | `metal_join.rs`; `facts/rows.rs` | Q67 counts them | Step 6 |
| 10 | A device fixpoint's total copied into a run's block at its end | `device.rs` (0325 step 4) | Each recursive stratum's rows | Step 6 |
| 11 | The transducer's packed batch: 44 bytes a node, and its child lists | `PackedBatch::nodes` (0293 "Interim") | Each matched batch | Step 7 |
| 12 | Findings, frames and memo entries copied into rows of their own | The output, frames and memo stores | Not yet measured | Step 8 (0325 step 7) |
| 13 | The transducer's residency: every tape it reads copied into slabs of its own, and documents copied between slabs | `residency::slabs::admit_docs`, `insert`'s `place`; `rerail_metal::slabs::gather` | Each tape a Metal batch reads | Step 7 |
| 14 | Found 2026-11-17: the line starts copied into the chunk block for the `line` and `Loc` views, and the spans past a document's nodes beside them; a guest's places made there. Both views read in place where the batch's tapes are on the device (step 4d); the copies are the fallback of a batch whose tapes are not, or with a guest placed by pieces or segments | `device_chunk.rs`, `fill_lines`, `fill_locs` | 4 bytes a line, 8 a span, each batch that makes the views | Step 4 |
| 15 | Found 2026-11-18: a parser's tape copied into its room (`place_stored`); a queued tape not in a room read into a vector (`queued_image`) | `tape.rs`, `place_stored`; `facts/core.rs`, `queued_image` | 4.1 MB a run on hogql 16z | Step 4a |
| 15 | Gone (step 4b, 2026-11-18): a 16z tape's blocks laid out as format 16. `DocTape::in_place`'s copy went first (the CPU reads 16z in place); then the transducer's residency's (`unblock_into` into its slab, for the kernels: the label and program kernels read the stored bytes through the batch's cache), and a 16z batch's views made on the device (they are read in place through a cache). `unblock_into` is deleted. | — | — | Done |

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
     to 0. Built (2026-11-18): `rerail_prof::copies`, a `static Site` at each marked copy (its row here and the step
     that removes it), bytes and calls always counted, CPU timed when the profile is on; the profile's heading
     `== copies (decision 0353)` lists every site that copied in the run. Row 12 has no site yet: step 8 counts it
     first. The guard reads the residency's directories too.
     hogql, 16z, the Metal binding, cold (load 38): 792.4 MB and 0.90 s of CPU in all. Row 4, views expanded on the
     device, 472.1 MB (the axis's, with row 6's 143.0 MB of pairs in the chunk: step 5); row 3, the chunk's node
     columns, 71.8 MB and 0.26 s; row 7, the per-node rows, 13.5 MB and 0.46 s; rows 8 to 10 (step 6), 71.8 MB; row 5,
     uploads, 9.0 MB and 0.15 s; row 13, tapes copied into slabs, 6.8 MB; row 15, 4.1 MB. Row 11 copies nothing in
     this run (its batches are resident).
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
   - First, each tape's one home is a residency slab (user, 2026-11-17). A batch's tapes were host-heap images
     (`Facts::tape`, an `Arc<TapeImage>`), and the device's were copies of them in the transducer's slabs
     (`slabs::insert`, and `gather`'s copies between slabs). The tape store's load writes a tape straight into a
     slab, a GPU-shareable chunk block, and the host's `DocTape` reads it there, holding the slab. A format 15 tape
     is decompressed (zstd, on disk only) into its slab; format 16 is placed as stored; a 16z tape stays compressed
     (its LZ4 blocks as stored), so the reads written once read 16z in place, a block decoded where a read falls in
     it, and `DocTape::in_place`'s laid-out copy goes. A host without a device uses the same blocks.
   - Its parts, each committed with its check (2026-11-17):
     - **4a.** Tapes placed in slabs and read there by the host: `Columns15`, `Tape16` and `Tape16Z` hold their
       bytes in a slab room, which the store's load decompresses (a zstd frame) or reads (a segment) into;
       `residency::slabs::admit` and `admit_docs` find a placed tape and copy nothing. A parsed tape is written into
       its room. Check: every format's reads equal the heap image's.
       Done for the facts' loads (2026-11-17): a stored tape's frame is decompressed straight into its room, and a
       parsed tape's stored bytes are written there, which the store's writer takes (`TapeBytes`); the parser's own
       columns remain until a parser writes into the room (`copy (0353 step 4a)`). hogql cold: 976 tapes in one
       slab, findings 1,629, peak 800–864 MB against 864–921 MB. Found on the way, unlisted: the transducer's
       residency copies every tape it reads into slabs of its own (`residency::slabs::admit_docs`, `insert`), and
       `gather` copies documents between slabs; row 13 below, step 7's.
     - A batch's tapes lie in several slabs, which no offset from one block reaches. A document's descriptor names
       its slab and its tape's offset there, and each backend gives the reads the slabs' bases: the host its
       pointers, the device their GPU addresses in an argument buffer (each slab made resident for the stage).
     - The views made on the device, by view (hogql cold, 2026-11-17, the profile's `device.view_made.*`):
       `node` 415.5 MB, the axis 362.7 MB (step 5's), `child` 75.9 MB, `col` 20.8 MB, `line` 20.8 MB, `docof`
       15.0 MB: 911 MB in all (809.1 MB in the upload line, which counts a view once). 4c and 4d go view by view,
       `node` first.
     - The view reads' family (4c): its pointers are the chunk block (`t`, the document's entry an offset from it),
       and the tape's base and view (`tt`, `v`), which the kernel or the host binds from the entry and the slabs'
       bases. A view read calls the numbering's reads with `t` and the tape's with `tt` and `v`. The entry holds the
       projection's numbering's offset, the tape's slab and offset, its `TapeView`, and its code table's kind and
       field per code (a derived table, small, beside the dialect's canonical codes): no per-node column.
     - **4b.** 16z read in place by the reads written once (a block decoded where a read falls), on the CPU, then
       the kernels; `in_place`'s laid-out copy and the slab's `unblock_into` go.
       Decided (user, 2026-11-18): decoded per reader, nothing outliving the read's slot. A 16z read is format 16's
       read of the block it falls in (its decoded bytes, the block's view, the node's place in it), as `F16Z` reads
       today, and a parent's walk crosses blocks; the reads written once do the same, the block decoded on a miss
       by an LZ4 block decode written once in CLIF (so the kernels have the same code), into the reader's slot. On
       the CPU a thread's slot (two blocks, as `F16Z`'s cache). On the device, a threadgroup decodes its rows'
       block into threadgroup memory, spilling to device memory past about 16 KB.
       - Probes, answered (2026-11-18): a stage's probes read a view at any row, so a threadgroup's reads are not
         within one or two blocks as a scan's are. They need no slot of their own: every read, a scan's or a
         probe's, goes through the batch's shared cache (below), which is sized to hold the batch's blocks. hogql
         cold, 16z on Metal, statistics on (`RERAIL_ZCACHE_STATS=1`): 65 batches, 2,992 decodes for 3,342 blocks
         (each at most once), 24.3 M hits, 1.8 M lookups that waited for a block being decoded, no duplicate, no
         turn, no failure; findings 1,629.
       - The CPU's part (2026-11-18): a 16z tape's view (`F16Z_FORMAT`, `Tape16Z::view`) names its block index,
         its blocks' side lengths and its compressed bytes where they lie as stored, its id, and the cache it reads
         through (by its address less the tape's, and its entry size). A format 16 read of a 16z tape
         (`by_format`'s third case) calls `tape_z_block` (cold), which finds the node's block in the cache or
         decodes it into the cache's head (LZ4's block format in words: 8-byte copies, an offset under 8 widened
         as LZ4's reference decoder does, a length's extension bytes one a step), and the read reads the entry as
         format 16 with the node's place in its block, every load masked to the entry; `parent` walks back over
         blocks' starts by `tape_slot`, and a call from the block's read finds its block itself. `DocTape::in_place`
         gives the stored bytes (row 15's laid-out copy goes). The CPU's reader (`view::Reader`) is the view and a
         cache of its own after it (two live generations of two blocks), and the CPU executor gives each evaluation
         that reads a 16z tape one, since an evaluation's tasks run in order on one worker.
       - The cache is the kernels' (below), checked on the CPU first: eight threads share one cache of four
         blocks a generation over a tape of more blocks, so it turns and copies all the time (0.9 M misses, 0.4 M
         turns in a run), and a read is wrong only where the cache counted no room. Found by that check and fixed:
         a reader checks its entry's key and completion before it reads and its completion after; a writer claims
         its entry by a compare-and-swap of its completion (marked as being written in its epoch), and skips one
         an older writer still writes, so no two write one entry.
         hogql cold with `RERAIL_TAPE=16z`, against the commit before: findings 1,629 throughout; the CPU's
         binding, 4 pairs at load 30–49, peak footprint 397–472 MB against 429–456 MB, 121.7–143.5 B instructions
         against 119.7–128.3 B (noise); the Metal binding, 1 pair at load 45, 102.7 B against 112.9 B, peak
         1,207 MB against 1,289 MB. Checks: the JIT's reads of a 16z tape of 6 blocks equal `F16Z`'s, node by
         node, and the JIT executor's outputs over one equal format 15's.
       - The tape crate's `F16Z` still decodes with `lz4_sys` into its thread's two blocks, then reads them with the
         generated format 16 reads: the same block, decoded by a second decoder. Built once (f9451f0dd: `tape_lz4`,
         the CLIF decoder translated by the Rust backend, its slot's loads bounded by the decoder) and taken out
         again the same day: decision 0399 (the Oscar worktree) rewrites 16z as four pieces a block, and this
         branch stays off the 16 and 16z paths until it lands. The one decoder is redone on 0399's form then.
       - The device's views read a 16z batch in place (2026-11-18; `views.refused.z_on_device` gone): the join
         kernels include the reads with the 16z case (`metal_tape_reads_z.metal`); a batch whose tapes are 16z
         gets one cache (`device_chunk::zcache_spec`), which every stage that reads its views shares, resident
         to be read and written; each 16z row's view names it, its entry size the cache's (a view's loads are
         masked to it). A generation holds four times the batch's blocks, 64 at least, within an eighth of a
         chunk's budget (`RERAIL_ZCACHE_MB`, 48 MB): with one entry a block, the device test's batch of five
         blocks turned 25 times in one pass and 8 reads found no room (the dead entry read: wrong ends); with
         the room, 5 misses, no turn, no failure. On hogql (24 batches of about 136 blocks, 544 entries a generation,
         8 KB entries) two more faults showed, both fixed: waits of 1,024 loads ran out while a block was
         decoded on a loaded device (11 M duplicates; `SPIN` is now 65,536), and a thread that missed took its
         entry before it raced for the block's index slot, so the thousands that missed one block at once took
         thousands of entries and gave them back (the head turned 111 times a batch, 330 M reads found no room).
         Now the slot is claimed first: an index slot holds the key's tag (16 bits) above its entry's number,
         0 while its writer has not taken one, and a thread that finds its tag pending waits on the slot; only
         the slot's winner takes an entry. Then hogql's 46 batches: 1,880 decodes for 2,769 blocks, 18.9 M hits,
         1.4 M waits, no duplicate, no turn, no failure.
       - Found on hogql and fixed (2026-11-18): with the join kernels on the reads with the 16z case, a format 15
         run on Metal lost findings (1,599–1,627 against 1,629, a different set of files each run; none extra),
         though no 16z tape was in it, the format 15 code of every read was the plain one's line for line, and
         the device's views over 22 real files were the host's rows on every try. With the plain reads in the
         join kernels, 1,629 three times; with the view read (`vread`) compiled as a call
         (`__attribute__((noinline))`) and the reads with the 16z case, 1,629 three times. Inlined, every stage
         kernel that reads a view carried every view's reads with their 16z paths; what in the Metal compiler
         or the device made those kernels lose rows is not known. The block's decoder (`tape_z_block`) is a
         call too (`msl::COLD`), and a join pipeline that cannot run its dispatches' groups of 256 threads is
         not made (`join::GROUP`; none was refused).
         hogql cold, 16z on Metal, the commit before this one's engine and this one's, at load 28–41: findings
         1,629 in every run; 102.7–107.4 B instructions against 119.6 B (the views of 31 batches read in place
         where they were refused and made); 31 batches, 1,873 decodes for 3,006 blocks, no failure. Format 15:
         1,629 three times, 73.1–90.1 B instructions against 66.2–74.3 B (noise at that load). A stage whose cache's failures rose while it ran is refused
         (`Run::failed`), and the stratum runs on the CPU. The cache's counts go to the profile (`zcache.*`)
         when the batch's directory goes, the statistics mode's with `RERAIL_ZCACHE_STATS=1`.
       - A 16z tape is stored as it is (2026-11-18, the user: "the 16z format is designed to be able to decompress
         blocks, not tapes. I don't want to pay for a full tape decompression, just blocks."): its frame in the
         store is its bytes (`CODEC_RAW` inline, as in a segment), never a zstd frame of the whole tape
         (`compress_tape`, for a tape the engine keeps as it is: `set_tapes_kept_as_is`, MAGIC16Z). A load is
         then the exact bytes, and a reader decodes only the blocks it reads. The other formats keep zstd frames.
         Check: `the_device_reads_every_format_as_the_document_does` reads each tape's codec in the store.
       - The transducer's kernels (done 2026-11-18): the residency places a 16z tape as stored (`place`: its bytes
         as the store holds them, no decompression since a 16z tape's frame is raw), its `DocView` the tape's 16z view with its columns
         where the tape lies in the document. A batch's table (`DocTable`) makes one cache for its 16z documents
         (`zcache_for`, sized as the views' directory's, `Spec::configured`) and points each view at it; a table
         whose 16z documents could get no cache is refused (a view's cache would be its own tape's bytes). The
         label, pattern and interpreter kernels read the z reads; each encoder makes the cache resident to be
         read and written; a command buffer after which the cache counted a failure fails its batch, which the
         wave splits (`capacity`); the counts go to the profile when the table goes. `unblock_into` (row 15) is
         deleted. Checked: `the_device_reads_every_format_as_the_document_does` (16z read on the device, no
         failure), the matcher's Metal tests, hogql.
         - Found: a label kernel's compile. With every read's 16z body inlined at each call site, the first run
           of each format took 220–300 s, almost all of it the Metal compiler: `label_micro` alone 18 s at
           format 15 and 33 s at 16z, against 1.8 s with format 15's and 16's reads; the interpreter 113 s. The
           function constant folds the 16z branches only after the backend's costly work. Now the kernels' z
           source calls each read's 16z body apart (`<read>_z`, the read built for `F16Z_FORMAT`, noinline;
           `reads::build_split`), and a batch of format 15 or 16 compiles the reads without 16z at all
           (`tape_view`). Cold: `label_micro` 3.0 s at 16z, the interpreter 8.5 s against 4.9 s; hogql's first
           16z run compiled for 38 s in all (16 pipelines). Measured with the kernels' sources as the engine
           compiles them (`RERAIL_METAL_DUMP=<dir>`) and `scripts/dev/metal/compile-time.swift`; each compile's
           time is in the profile (`metal.compile_ms`). The system's shader cache keeps a compile by its code: a
           comment added to the source still finds it, so a cold compile changes a literal.
         - Found: a shared reader whose tries ran out read its entry without counting a failure, so a read could
           be wrong unseen; the CPU's stress test passed only while the failures it counted outnumbered the
           wrong reads (about 450 wrong of 1.28 M reads in every run, failures 274–879). Now a read that gives
           up counts a failure, before its body and after it: about 2,000 failures against the same 450 wrong
           reads, in every run.
         - Found: the main thread's wait for its next chunk spun (263 s of CPU in a 280 s wait on a compile):
           its poll advanced the builder, which rang the bell it waited on whether or not anything moved. It
           rings only when a step moved (`corpus::Builder::advance`).
         - hogql cold, the Metal binding, profiling builds, load 8–55: 16z, findings 1,629 in 72 runs (one more
           run panicked, Q125), 99–117 B instructions, median 105 B; format 15, 1,629 in 32 runs, 65–75 B,
           median 73 B (the runs that compiled the kernels apart: 135 B and 93 B). Statistics on: 2,187 decodes
           for 2,820 blocks, 18.7 M hits, no failure, no duplicate.
       - The kernels' part, decided (user, 2026-11-18): "a generic rolling GPU cache to amortize the cost": an LRU
         cache, kept with atomics, of 2–4 generations, each a hash index and an arena. A lookup looks across the
         generations, the head (active) and the frozen ones behind it. A miss computes its value (here a 16z
         block decoded) into the head. When the head fills, the oldest generation is dropped and becomes the new
         head. A hit in the oldest generation is copied into the head, so the cache keeps what is read (LRU). It
         makes 16z viable for large runs on small amounts of GPU memory. Its shape, as built:
         - Generic: a key (a u64; for a 16z block, its compressed bytes' address, unique across tapes) to an
           entry of fixed size (a power of two), written once in CLIF for the JIT and the kernels. The CPU's
           reader is the same cache, small and of one thread.
         - A spare generation, cleared ahead: of G, G − 1 are live; a rotation is one compare-and-swap of the
           epoch, and the thread that wins clears the next spare.
         - Misses on the device, decided (user, 2026-11-18, superseding "a miss never waits"): claim first, a
           bounded wait. Without waiting, the device's threads that miss one block at once each decoded it (57
           decodes for a tape of a few blocks in the view probe); at four blocks a generation their copies filled
           the head, the cache turned on itself, and 75 lookups found no room. Now a miss claims an entry, writes
           its key and indexes it before it decodes; a lookup that finds its block being written waits for it
           (`SPIN` loads at most), then decodes it too (a duplicate, counted). A writer whose index slot is taken
           by the same key gives its entry back and looks again, so it finds the other's and waits. No wait is
           unbounded: lanes of one SIMD group are not promised to make progress apart, so a wait may run out.
           The user's further asks: a cache of a reasonable size, so its hit ratio is good (the device's is sized
           from the budget, to hold the batch's blocks where it can); if that is not enough, the threads' start
           points randomized, so they do not all miss one block at once; and a mode in which the cache keeps its
           statistics, for tuning (`Spec::with_stats`: hits, waits, duplicates, beside the misses, copies, turns
           and failures it always counts; `cache::Counts`).
           Measured on the device's view probe (statistics on): each tape decoded once (1 miss, against up to 57
           before), no failure and no duplicate at 1, 4 and 64 blocks a generation, hit ratio 0.993–0.999 (16–542
           lookups waited, every wait answered), the rows the host's in every case. On the CPU, eight threads over
           a cache far too small for their tape (four blocks a generation): hit ratio 0.77, 0.77 M lookups waited
           instead of decoding. (That a read was wrong only where a failure was counted held by chance: below,
           "a shared reader whose tries ran out".)
         - A read from a frozen generation that is dropped while it reads is caught: the read checks its
           generation's stamp after it, and reads again. An entry's loads are masked to the entry, so a read of
           one being rewritten reads nothing outside the cache.
         - Every loop is bounded, as the device's work queue's are. A read that finds no room (the spare not
           yet cleared, its retries spent) counts a failure in the cache's header; the host refuses that
           dispatch's batch, which runs on the CPU (counted), rather than wait.
     - **4c.** The views' reads written once (CLIF, for Rust, the JIT and MSL), over a document's `TapeView` and
       its projection's `Numbering`: each view's scan over 64-node groups (the open subtrees from one descent) and
       its probes (by id; by parent, first child and next numbered sibling). Check: their rows equal `Viewed`'s,
       for every view and format.
     - **4d.** A stage's input is a relation or a view, through one read (a kernel template over it). The batch's
       descriptors are in its chunk block, the tapes in their slabs, reached by GPU address (no `gather`).
       `expand_view`, `VIEW_EXPAND`, the chunk block's node columns (row 3) and the residency's view entries go.
       Check: `device::parity` for every view.
       Done for `node`, `child`, `field` and `core_node` (2026-11-17, 4c and 4d):
       - The reads: `view_node`, `view_child`, `view_field`, `view_core` (`rerail_clif::view_reads`, a third family
         of reads: the entry, the tape's base, its view). A row is a point read: its tape node by `num_select`,
         its columns off the tape, a kind or field by the entry's derived table of the tape's codes, a child's
         place by the walk's places (the derived column the host's point reads read, 0353 step 3). Checked on the
         JIT and the device against the host's rows, in all three formats, whole and narrowed
         (`the_view_reads_are_the_hosts_rows`).
       - The stages: a thread a row, its document by bisection of the batch's directory (`ViewDir`, `vread`); a
         probe by key reads an index built over the view's rows, in a relation of no columns (an index in its
         relation's block). Every reader of a relation reads a view the same way: a stage, a witness probe, an index
         build, `rows_copy`. Checked: `views_made_on_the_device_are_the_hosts_rows`, hogql's findings.
       - Not yet: the scans over 64-node groups and the probes by parent (first child, next sibling); the Rust
         backend of the view reads (the host's `Viewed` reads the same columns through `Facts`, by hand).
       - `line` (`View::Line`, 2026-11-17): a document's line starts are a derived fact written once where they are
         computed, by a chunked search of the terminators (`memchr`, no byte loop): `LineTable`'s u32 words `[n,
         starts[n], ends[n]]`, in a slab's room beside the tape for a loaded document (`LineTable::placed`). A fourth
         family of reads, the lines' (`lines_line`: the starts at or before a byte, by bisection), which a view read
         calls with the lines' base, its fourth pointer; `ViewRow` holds the lines' GPU address. A batch where a
         document's lines are not in a slab makes `line` (counted, `views.refused.lines_unplaced`).
       - core.rr's places (`Loc`: `line`, `end_line`, `col`, `docof`, 2026-11-17): a row is a position (a node, a
         token, the whole text, a line, an end point), its span off the tape or off core.rr's lines, which are a
         derived fact of the projection, written once into its block by a chunked search (`core_rr::Lines`' words
         `[ns, starts[ns], ne, ends[ne]]`; the byte loop goes). The entry holds the positions' counts and a run to the
         lines; `loc_line` (a relation's read of the same body as `lines_line`) finds a line. A guest's places are
         its host's lines through its placement, a copied stretch or a point, from the entry's scalars (decision
         0055). A normalized string's pieces and an embedder's segments live in no block: such a batch makes its
         places (counted, `views.refused.locs.guest_*`). On hogql every guest is a copied stretch.
       - Left made: the axis (step 5); a batch whose tapes are not on the device (the device's budget spent); a
         batch with a guest whose placement is not a copied stretch or a point. `expand_view`, `VIEW_EXPAND`,
         ChunkDoc's node columns and `fill_lines`/`fill_locs` (row 14) stay for them.
       - Found on the way: the slabs holding tapes were never retired (4d's first part); now they are retired once
         only the pool holds them, so a long session frees them with its tapes.
       - hogql cold, 3 pairs: views made 347–503 MB against 597–788 MB; `node` and `child` 0. With `line` (whose
         rows hogql's core.rr shape does not read: its `line` is `Loc`'s), 3 pairs against 4d at load 7–9: 67–78 B
         instructions against 77–83 B, findings 1,629. With `Loc`, 3 pairs at load 23–26: views made 341–502 MB,
         the axis alone (against 397–524 MB with `Loc`'s 70–98 MB); `Loc` read in place, 4.1–5.9 M rows a run;
         74–85 B instructions against 68–89 B; findings 1,629.
     - **4e.** Measured: the 660.8 MB is 0; refusals and strata apart, against Q68's.
       Measured 2026-11-18 (hogql cold, format 15, the Metal binding, 3 runs at load 45–57, after 4b's CPU part):
       findings 1,629; the views made on the device are the axis alone, 439, 523 and 467 MB (step 5's: of the
       660.8 MB, every view step 4 covers is read in place). Strata placed on the GPU and refused at run time: 11
       strata in 32 runs (0.56 s on the CPU), 7 in 25 (0.19 s), 2 in 2 (0.01 s); the largest refusals are the axis's
       views of 2.2–2.5 M rows (`ann_ctx`). Strata run apart because the wave's budget was spent: 73, 134 and 77,
       against 197 and 127 at the `line` commit and Q68's 103–181 (36–55 under the old prices). Instructions 65.2–
       76.8 B. What remains of both is the axis's made views, which step 5 writes once.
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
   - Built 2026-11-18, for the batch's solve (`nearest`, `inside`, `within`):
     - The relation lies alone in a run's block (`Part::for_writes_indexed`), not the chunk block: a derived fact
       like a rule's head, with room for an index by each column (strata probe an axis by `n` and by `a`). In the
       chunk block, or in a block of its own sized before the count and leased from the device's budget (a first
       cut: containment's room 8 rows a node), it filled the budget: refusals rose from 5–38 to 23–52 a run.
     - On the device: `axis_count` and the scan (one submission), the total read, then `axis_write` writes each row's
       two ids (`global_of`) through `view_put`, the writer written once, bounded by the relation's chunks
       (decision 0403). The starts (node row → first row) stay in the chunk; on the CPU, a vector beside the
       relation.
     - On the CPU: the climb counts, then writes `batch.id` of each side into the same kind of relation.
     - Every reader reads it in place: `value` and `documents_of` through `Part::get`, the device through
       `Source::in_block` (`DevRel::in_block`). `Within::device_view`, its `pairs` and `on_device`, `view_expand`'s
       axis branch, `DeviceView.pairs` and `expand_view`'s argument for it are deleted; view number 4 stays unused.
     - `has` is left: its probe that climbs the tape on the device is a probe read, done with 4b's parent probes.
   - Measured (hogql, 16z, the Metal binding, cold; interleaved with the commit before, 3 pairs at load 23–33):
     copies 125.9 MB a run against 792.4 MB (rows 4, 6 and the axis's row 9 at 0: 472.1, 143.0 and up to 124.5 MB);
     strata refused at run time 2, 2 and 2 against 22, 10 and 5; peak 817–928 MB against 831–1,023 MB; instructions
     106.1–113.0 B against 103.1–108.2 B; findings 1,629, and 1,629 in format 15.
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
