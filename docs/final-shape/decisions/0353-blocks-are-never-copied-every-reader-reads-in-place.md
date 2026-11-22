# 0353: Blocks are never copied: every reader reads the data where it lives

Status: decided (2026-11-17), by the user: "the entire point is that the CPU and GPU kernels all operate on exactly
the same linear data"; "it must be clear EVERYWHERE that blocks NEVER get copied into different data-structures for
'convenience'". The plan below is being built. Supersedes the parts of 0289 §1–2, 0293 §6 and 0325 step 5 named in
"What it replaces". Follows 0140 (the tape is the engine's bytes), 0266 (blocks), 0293 (one tape API, read in place),
0310 (reads written once, three backends) and 0325 (relations are blocks).

Amended between memory domains by 0440 §7.2 (the user, 2026-11-18): within one domain (a host's RAM and its unified GPUs) this holds as written; between domains (a discrete GPU's memory, another host) a block is copied as the same bytes, never into another data structure, and the engine keeps the copies coherent.

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
times a run against 36–55 (0325's Built entry, Q86).

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
   - **The memory hierarchy.** Registers, caches, TLBs, paging, a GPU's own memory and caches, and a driver that
     holds a device's image of a buffer and makes it current by a map, a sync or a migration: each holds the data
     the processor is computing on now. None of them is a second home that readers read instead, and none is the
     engine's layout. They are how hardware reads memory.
6. **What this rule is about: the engine's data design.** It forbids the engine making a second data structure
   (another layout, another home) for a reader. It is not about hardware or drivers. Never use it to refuse a
   device, a backend, a driver or a platform. A device whose memory is not the host's is used, and the backend
   moves the bytes as that device needs (the OpenCL backend's `synced`, 0440). When in doubt, ask one question:
   does the engine's code now keep the data in a second structure of its own that readers read instead? If not,
   it is not a copy.
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
matches, comments, `rx_span`) have their byte ranges only in `Extra`. Q216: those ranges written once into the
batch's block, and the view reading them past its own positions, so nothing makes a node's place.

**Step 3's other relations of row 7** (2026-11-17), each checked against "Not copies":
- `visit_index` (CPython's visit order), `host_node` (a guest node's innermost host node, an axis's answer) and
  `doc_root` (a document's root id, one row a document) are derived facts, each made only where a program reads it
  (`tree::Tree::new`, `GuestRows::document`'s `wants`). They stay relations.
- `intval` and `op_t` are read by no core.rr library rule; in core.rr's shape `NodeRows` runs only where a program
  reads them. In T's shape they come with `NodeRows`' eager `text`, `name` and `str`, deferred with Q121.
- What row 7 keeps: `core_rr`'s and `NodeRows`' eager derived values (`text`, `name`, `value`'s literals, T's
  rows), deferred (Q121), and the nodes' place rows beside spans (Q216).

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
| 3 | The chunk block's node columns: `core`, `up`, `fielded`, `low`, `start`, `end`, `kind`, `field`, and the canonical code copy. `low` is made by one scan of the granule table. `core`, `up` and `fielded` are made here from the rank index and the walk, now that the host holds no arrays of them. | `device_chunk.rs`, `build` (`made`, `made_in_order`, `put`) | 0: gone | Done (step 4, 2026-11-22) |
| 4 | Views expanded into device relations | `metal_join.rs`, `expand_view`; `device.rs`, `resident` | 0: gone | Done (step 4, 2026-11-22) |
| 5 | Views and flat relations uploaded from the host. Symbols' names no longer (0478 step 2e, 2026-11-19: every text kernel reads them through the atoms directory; `texts` and `DevText` are deleted). `tape`, `ast_depth` and `node_order` are read in place since 2026-11-20 (`view_tape`, `view_depth`, `view_order`, written once in CLIF: a node's parent, its numbered nodes under it by `view_end`'s rank, its depth by a climb, its order key by `num_order_of`): on the M3's 300-file run they were 69.8 MB (`ast_depth`) and 13.7 MB (`tape`) of 176.3 MB uploaded (ab111). Each upload is counted by why it was not read in place and by its columns (`device.upload (...)`) | `metal_join.rs`, `DevRel::upload`; `device.rs`, `resident` | 0: gone. On cold hogql its last 39 calls were relations of no rows | Done (step 6, 2026-11-22) |
| 6 | Gone (step 5, 2026-11-18): an axis's pairs copied into the chunk, then expanded to ids. The solve writes ids once into a relation of its own, which every reader reads where it lies; `put_pairs` and `VIEW_AXIS` are deleted. `has` probed by its node or its ancestor is walked on the tape on the device (6fdbc1eb2, `DevRel::has`); read whole or by both columns it is still uploaded (row 5). Measured (ab98, 2026-11-20, the M3, 300 files, `--metal`, 1 GB, load 5–9; each arm with 960374749f's planner fix): 463 violations each run; instructions 544 G without the walk, 554 G with it (means of three runs, within their spread of about 30 G), wall 24 s each; `has` walked 6 times a run; row 5's uploads rose from 312.1 MB in 863 calls to 454.3 MB in 949, since strata a keyed `has` kept on the CPU now run on the device and upload their other inputs | — | — | Done |
| 7 | `core.rr`'s and the vtab's per-node rows: tape answers (`line`, `end_line`, `col`, `docof`, `visit_index`, `doc_root`, `host_node`) and eager derived values (`text`, `parts`, `words`, `intval`, `op_t`). In core.rerail's shape a node's `text` is read in place (2026-11-22); left: T's shape's `NodeRows` (Q121) and `text` of a span or of a node only a cross-file relation binds | `syntax.rs` `node rows`, `NodeRows`; `core_rr`; `tree.rs`; `guests.rs` | 0.0 MB, 0.3 ms on cold hogql (0.45–0.49 s of CPU a run before) | Step 3 |
| 8 | The CPU's head rows, staged in `Fresh`, merged by `extend_from_slice`, then pushed into the relation | `setfix.rs`, `Fresh` | 0: gone (21 MB in 972 calls on cold hogql before: push_fresh 15.6, absorb 5.3, the device's staged 0.1) | Done (step 6, 2026-11-22) |
| 9 | A relation of many parts merged for the device (`in_block`). The key's copy is gone (step 6, 2026-11-22): a key past a block's room refuses its stratum, and a relation alone in a block has room for its whole directory. Before: a second key's index copied the relation (`DevIndex::moved`). Since 0384 (steps 1–8, 2026-11-18) a relation is one relation or a chain of continuations read in place, so the merge is left only where a part is appended after another relation's rows (`append_part`, counted), with a chain's newest two segments compacted past `MAX_SEGMENTS`. A recursive component's deltas on the device are made with room for an index of each key its plans probe them by (`Own::keyed`, `Part::for_device_indexed`, 2026-11-20), and its totals grow in a block with room for theirs (row 10). That was not most of the row: on the M3's 300-file run it copied 852.4 MB in 279 copies before (ab108) and 901.8 MB in 322 after (ab109). The site now says where the relation copied lies, the device's own (an upload, a delta) or in a block of the run's with no room left, and the device's report counts each (`moved`): 766.4 MB in 247 copies were in a block with no room (ab110). A head relation the device writes of 4,096 rows or more is now alone in a block with room for its indexes (`head_room`, `Part::with_index_room`, anonymous memory taken as written): 621.7 MB in 114 copies left (ab111), each about 5 MB, of relations not yet told apart | `metal_join.rs`; `facts/rows.rs`; `device.rs` | Q85 counts them; 0 merges on cold 300-file runs; 0 on cold hogql since 2026-11-22 (8.3 MB in 2 before: the axis relation, room for 2 indexes, probed by 3 keys) | Step 6 |
| 10 | Gone (2026-11-20): a device fixpoint's total copied into a run's block at its end, and each round's total made anew with the last round's rows copied into it (rounds × rows). A set head's relation is now one relation in a block of the run's life from its first round, each round's new rows written past its rows, its indexes forgotten as it grows, taken by the store as it lies (`SetHead`), as a lattice head's is (decision 0384 step 5). Measured before: 187.4 MB in 225 copies at the end on the M3's 300-file run (ab109); the rounds' copies were not counted | `device.rs` (0325 step 4) | 0 | Step 6 |
| 11 | The transducer's packed batch: 44 bytes a node, and its child lists | `PackedBatch::nodes` (0293 "Interim") | Each matched batch | Step 7 |
| 12 | Findings, frames and memo entries copied into rows of their own. Counted since 2026-11-22: a granule's rows gathered into its kept entry (`retain.rs`); an input's documents read into a table or a layout, and a kept entry's or a documents' table's words read for their decoders (warm runs); and the writer's columns, a kept relation's or a granule's rows read into columns for the packer (`kept_facts::columns`), which the store's writer makes after the profile is reported | The output, frames and memo stores; `retain.rs`; `kept_facts.rs` | 216.0 MB in 21,643 calls on the M1 Max's 300-file run (`--metal`), all the granules' gather; 6.1 MB in 1,539 on cold hogql | Step 8 (0325 step 7) |
| 13 | The transducer's residency: every tape it reads copied into slabs of its own. (Documents copied between slabs, `gather`, are gone, 0471: a batch reads each document in its own slab, by the slabs' GPU addresses.) | `residency::slabs::admit_docs`, `insert`'s `place` | Each tape a Metal batch reads | Step 7 |
| 14 | Found 2026-11-17: the line starts copied into the chunk block for the `line` and `Loc` views, and the spans past a document's nodes beside them; a guest's places made there. Both views read in place where the batch's tapes are on the device (step 4d); the copies are the fallback of a batch whose tapes are not, or with a guest placed by pieces or segments | `device_chunk.rs`, `fill_lines`, `fill_locs` | 0: gone | Done (step 4, 2026-11-22) |
| 15 | Gone (step 4b, 2026-11-18): a 16z tape's blocks laid out as format 16. `DocTape::in_place`'s copy went first (the CPU reads 16z in place); then the transducer's residency's (`unblock_into` into its slab, for the kernels: the label and program kernels read the stored bytes through the batch's cache), and a 16z batch's views made on the device (they are read in place through a cache). `unblock_into` is deleted. | — | — | Done |
| 16 | Gone (0446, 2026-11-19): a label level's fact rows, uploaded as `(node, label)` pairs for the pattern kernels (`Pass::Fact`). The host writes the findings once into the batch's fact block, and the CPU and the kernels read them there | — | — | Done |
| 17 | Found 2026-11-18: a parser's tape copied into its room (`place_stored`); a queued tape not in a room read into a vector (`queued_image`) | `tape.rs`, `place_stored`; `facts/core.rs`, `queued_image` | 4.1 MB a run on hogql 16z | Step 4a |

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
     uploads, 9.0 MB and 0.15 s; row 13, tapes copied into slabs, 6.8 MB; row 17, 4.1 MB. Row 11 copies nothing in
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
       - One decoder (2026-11-18): the tape crate's `F16Z` decodes a block with `tape_lz4`, the reads' LZ4 decoder
         (`z_lz4`, which `tape_z_block` inlines for the JIT and the kernels) translated by the Rust backend, into its
         thread's two blocks, then reads them with the generated format 16 reads. `lz4_sys` only compresses (the
         writer). The Rust backend checks a load against the tape's bytes when it is read-only and not aligned (the
         tape's own); the decoder's loads of its slot (a match's copies) are bounded by the decoder itself
         (decision 0463), and a read that takes an address (`tape_lz4`'s destination) is `unsafe`. The kernels'
         UKL leaves `tape_lz4` out (`HOST_ONLY`). Taken out once (f9451f0dd's first form) while 0434 rewrote
         16z, and redone on 0434's form (rr-align's roll-up 7, merged here 2026-11-19). Check:
         `the_reads_decoder_decodes_what_lz4_compressed` (400 blocks of literals and matches at every offset, a
         block cut short); hogql 16z, findings 1,629.
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
         store is its bytes, never a zstd frame of the whole tape. A load is then the exact bytes, and a reader
         decodes only the blocks it reads. Since 2026-11-19 every tape and source, of every format, is stored as it
         is in a segment, and no row holds bytes (0295 step 5, floor 7). Check:
         `the_device_reads_every_format_as_the_document_does` reads each tape's row in the store.
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
           run panicked, Q265), 99–117 B instructions, median 105 B; format 15, 1,629 in 32 runs, 65–75 B,
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
       - Over 0336's 16z (the hub's merge of Native-and-GPU-strata, 2026-11-18). The hub's 16z is not this branch's
         (one LZ4 block of a block's whole layout, a block index, its side lengths): it keeps format 16's header and
         part starts, its checkpoints and newline index raw, and stores each block as four pieces, an array's bytes
         for the block's nodes then its side list's part, each raw, one LZ4 block or a repeat of an earlier piece
         (0336 §4, §5). The cache, its entries and the reads are this branch's; what knows the layout is ported:
         `Tape16Z::view` names the piece index's starts, kinds and payload (`columns[0..3]`) and the header's part
         starts (`parts`), and `z_decode` fills a block's entry as the tape crate's `F16Z` decodes it: each side
         length from the part starts, each piece copied (raw, or a repeat's earlier piece) or decoded by `z_lz4`
         into a scratch past the block and copied from there into its array's region and its side list's, the
         checkpoints copied from the raw run (past the part starts, 8-aligned: `view` refuses a header where they
         are not), each region's room cleared. The entry holds the scratch (its largest piece) beyond the block.
         The scratch's two copies are the decode's own (an LZ4 piece decodes to one run, its two parts apart in the
         block's layout); a decode that writes each part where it lies would take the copies out. The residency's
         16z stays the hub's: the device decodes the pieces where a document is placed (0336 §8), and this branch's
         `unblock_into` (row 15, on the host) is not taken.
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
   - Measure: the 660.8 MB is 0, and the device budget's refusals and strata run apart, against Q86's.
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
       (decision 0463). The starts (node row → first row) stay in the chunk; on the CPU, a vector beside the
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
   - A second build of the same (benchmarks-on-integration, cd091ab09 and ec0835fc7, 2026-11-19: ids written from
     the solve's pairs on its first whole read) was superseded by this one when the branches merged; its device
     test is kept on this one (`device::tests::views_made_on_the_device_are_the_hosts_rows`, the device's read in
     place). Measured on the bench's full shape (the M3, a 512 MB budget, Q203 (c)): the residency's made views,
     142 MB of 511 at a refusal, gone; refusals 21–68 a run against 44–88; 463 violations in each. Without index
     room each device index copied the relation whole (`DevIndex::moved`, 16 MB at a refusal), as this one's
     `with_index_room` avoids.
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
9. **Placement is priced again** on what is left: no upload, view or copy term (0325 step 6, Q86).

The order removes the measured cost first: step 4 is the 660 MB. Steps 2 and 3 come before it because the GPU's
reads need the ids, and the CPU's reads are their check.


### Step 2's reads in joins (2026-11-17)

With the per-node arrays gone, a join step read each candidate's row a column at a time from the tape: its tape node
by a select of the rank index, its id by a search of the granule table, its parent (format 15) by a search on the
tape. On 50 Python files of B-now, format 15 cold, that was 340 G instructions against 315 G before step 2, with
the same findings. Three changes keep the reads where the work already is:

- A step reads only the columns it binds, tests or checks, and not a key column the probe matched exactly
  (`RowRef::read_need`).
- A probe reads its rows' values as it finds them (`Source::lookup_read`). A lookup by a node's id knows the row's
  id. A lookup by a parent knows the parent's id and each child's tape node. A lookup by kind or by tape node knows
  the tape node. None of these is searched again.
- The id and number conversions search less: `number_of` finds a slot from an interpolated guess (the slots are
  hashes), and `low_of` searches between two jumps, one every 64 numbers (4 bytes for each 64 nodes).
  Both searches are the CLIF reads' (`num_number_of`, `num_granule`): the jumps are a run of the descriptor
  (`Numbering::jumps`), so the host and the kernels search the same way.

A scan of a whole view reads its rows in one walk (`Source::for_rows_in`), as `for_rows` does.

After: 322 G against 314 G on format 15 (+2.6%), 319 against 315 on format 16 (+1.1%), 397 against 391 on 16z
(+1.4%), and the same findings on all three. What remains is in-place reading the arrays never paid for: on format
15 the parent search, and the select that finds the tape node of a node probed by its id.

### Step 3 on the CPU: the executor reads the device's directory (2026-11-21)

**The user (2026-11-21):** "we need to REMOVE all of the host-side CPU stuff. The CPU needs to run on the same blocks
as the GPU only. … NO CPU-SIDE RUST POINTER-BASED DATA STRUCTURES." And: "A single memo as part of execution is
fine, it can be done on both the CPU and the GPU. And caches are fine, so long as they are implemented in a way that
both CPU and GPU can use and share them."

**Where it stands.** The device reads the views in place (step 4d). The batch's block holds a directory (`ViewDir`):
a `ViewRow` for each document, with each view's first row, its numbering's entry (`ViewDoc`) and its tape. The
kernels read every view through the reads written once (`rerail_clif::view_reads`: `clif_view_node`,
`clif_view_number`, …). The CPU's executor reads none of this. Its views are `Viewed` over host structures of its
own:

| Host structure (CPU only) | What it answers | Its home in the block |
|---|---|---|
| `made::Docs` (`slots`, `keys`, `tags`, `given`) and `Projected`'s fields (`key`, `file`, `tag`) | A document by key or index, its projection | `ViewRow` and `ViewDoc` |
| `Batch::offsets` (`made::Offsets`, a vector a view) | A view's first row of each document | `ViewRow::rows` |
| `DOC`, `LOCATED` (thread-locals) | The last document found | A memo argument of the directory's reads |
| `of_tag`, `programs` | A tag's documents; a file's programs | A directory's ranges by tag and by file |
| `kinds` (`KindRows`) | The `node` view's rows by kind | An index in the batch's block |
| `given_rows`, `Shared` (`strings`, `at`, `atoms`) | Rows by relation; a group's strings | The run's relations; the atoms directory |

The kernels search the directory by hand too: four bisections with no memo, by row (`vread`, `child_next`,
`view_expand`) and by key (`by_parent`).

**The steps.**
1. **The directory is position-independent and always made.** Its addresses (`ViewDir::chunk`, `ViewDir::rows`,
   `ViewRow::tape`, `ViewRow::lines`) become offsets, and each executor passes its own bases: host addresses on the
   CPU, device addresses on the GPU. A tape held in a vector (no device) is a base of its own. The directory is made
   with the batch, on every platform, whether or not a device is open (no `placed::device()` gate).
2. **The searches are reads written once.** A document by row of a view, and by key, are CLIF reads over the
   directory. Each takes a memo: the document found last, tried before the bisection. The CPU passes the probe's
   or scan's cursor, and a kernel passes its own. They replace the four bisections in UKL and `Batch::doc` and
   `Batch::locate`.
3. **The CPU's views are the reads.** `Viewed`'s scans and probes read through `view_reads` (the generated Rust;
   the JIT inlines the CLIF) over the directory. Check: every view's rows equal `Viewed`'s, for every view and
   dialect, cold and warm, before `Viewed`'s own reads go.
4. **The host structures in the table go.** Each one is deleted with its last reader, and the table here shrinks
   to nothing.

Each step ends with findings unchanged (1,629 on cold hogql), instructions measured against the step before, and
Metal checked on the farm's Macs (the mini, then the M3, then the M1 Max, one test at a time).

**Step 1, done (2026-11-21).** The directory holds no address of its own:
- `ViewDir::at` is the directory's offset in the batch's block. An executor finds the block's start as the
  directory's address less it (`view_block` in the kernels).
- `rows` and `homes` are offsets in the block.
- A row's tape and lines are a home's number and an offset in it (`ViewRow::homes`, `tape`, `lines`).
- The homes table (`ViewHome`) is the one place a base lies. It gives each slab's, or each tape vector's, host and
  device address, and each executor reads its own column.

The 16z cache is a home too: a block of its own, which the reads of both executors share and write. Binding the
batch's block for writing would make the host wait on every pass that reads it. The cache offset in a stored view
(`TapeView::side`) is still the device's: the join kernels read the view where it lies, in global memory, and
cannot set it per read. The host's reads will set their own from the same homes (step 3).

`build_dir` no longer asks for a device. It refuses a batch only for a document in another block, or one with no
tape read in place. The device's address (`ViewsInPlace::dir`) is given only when every home has a device address.
The directory is made on its first ask. The CPU asks from step 3, so until then a CPU run makes none: cold hogql
still gives 1,629 findings, at 88.1 B instructions against 87.1 B before. Test:
`the_views_directory_is_the_hosts_too`. On Metal, the mini passes `a_views_index_is_in_its_block_for_every_thread`,
`views_made_on_the_device_are_the_hosts_rows` and `a_view_made_on_the_device_is_made_at_each_stratum`, and the M3
passes the first. A home's device base is `ViewHome::gpu`, since `device` is a Metal keyword.

**Step 2, the reads (2026-11-21).** The directory's two searches are written once: `dir_doc_of_row(view, row, memo)`
and `dir_doc_of_key(key, memo)` in CLIF, in the relation family, with the directory as the descriptor. The host's
twins, written directly, are `ViewDir::doc_of_row` and `doc_of_key`. Each tries the memo first and then bisects.
A stale memo costs a comparison, never a wrong document. The memo is a value the caller passes and keeps; it is
never thread or process state.

The kernels' three bisections by row (`vread`, `child_next`) and by key (`by_parent`) now call the reads. `vread`
takes its caller's memo: the copy kernel and `ix_bucket` share one across a row's columns, and every other caller
passes a fresh one, which costs what the bisection cost. `view_expand`'s search (`chunk_views.uglykl`) is over the
chunk's copied document array, not the directory, and goes with that copy.

Tests:
- `the_directorys_searches_are_the_jits`: the host's searches against the JIT's.
- `the_kernels_directory_searches_are_the_hosts`: the device's against the host's, for every row of every view,
  every key and keys of none, with each memo.

**Step 2, the CPU's searches (2026-11-21).** `Batch::doc` runs before documents are made (decision 0494), and
`Batch::locate` reads first rows made from the documents' counts alone. So the batch's one directory is written in
two layers:
- **The search layer.** Each row's key, written when the batch's documents are known (`made::directory`, from
  `Docs::new` and `Docs::grown`), and each view's first rows (`ViewRow::rows`, ten slots: the nine views of
  `VIEWS` and `child`'s sibling view), written once from the counts on the first read of any (`Batch::firsts`).
  The header's `ends` holds each view's rows in all. `made::Docs` keeps no keys of its own (`Docs::key` reads the
  row), and `made::Offsets` holds no vectors: `made::Firsts` reads a view's slot in the directory.
- **The reads layer.** Each row's entry, homes, tape, lines and view, and the header's homes and cache, written into
  the same directory when the device or the reads ask for it (`build_dir`), field by field beside the search
  layer's readers. A layered source's directory (`views_over`, a fixpoint's admitted values) is still made whole,
  apart, from its batches' first rows.

`Batch::doc` is `ViewDir::doc_of_key`, and `Batch::locate` is `ViewDir::doc_of_row`, on the CPU. The memo each
passes is still held in the `DOC` and `LOCATED` thread-locals: `Source::value` has no cursor to carry it yet. Step 3
passes the executor's memo through the scans and probes and deletes both.

Tests: `the_directorys_searches_are_the_jits` checks that the device's directory is the batch's (one directory, its
offset `Docs::dir_at`), and the lazy-probe tests (`a_probe_makes_the_document_it_names_alone`,
`a_batch_made_by_group_holds_the_eager_batchs_rows`) read the search layer before any document is made.

Cold hogql still gives 1,629 findings, at 89.4 B and 91.2 B instructions (step 1: 88.4 B and 88.1 B; this Mac at a
load of about 33). On Metal, the M3 passes `the_kernels_directory_searches_are_the_hosts`,
`a_views_index_is_in_its_block_for_every_thread`, `views_made_on_the_device_are_the_hosts_rows` and
`a_view_made_on_the_device_is_made_at_each_stratum`. The mini could not build: its Xcode license is not agreed to.

**Step 3, the memo (2026-11-21).** The `DOC` and `LOCATED` thread-locals are gone. `Batch::locate`, `doc`, `node`,
`loc` and `id` take the caller's memo (`&mut u32`, a document index for both searches, started at
`vtab::NO_MEMO`). A source's readers pass theirs through `Source::row_at_memo` and `value_memo`, and the trait's
own row-by-row scans (`for_rows`, `for_rows_in`, `rows_of`) keep one across their rows. Each of `Viewed`'s lookups
keeps one across its values. Readers that read one row at a time with no memo (`row_at`, `value`) search afresh.
Cold hogql gives 1,629 findings at 91.4 B and 90.0 B instructions (89.4 B and 91.2 B before), so the executor's
point reads (`RowRef`) carry no memo yet.

**Step 3, the views' reads (2026-11-21).** **The user (2026-11-21):** "why does step 2 need to use the JIT? Surely we
can make Rust code that reads the same data structure". So the CPU's reads are Rust written directly, not the JIT:
- `rerail_tape_format::view_reads`: `Tape` (formats 15, 16 and 16z) and `Doc`, a document's views as reads of its
  entry (`ViewDoc`), its tape and its lines where they lie: node, child, field, core, line, the four `Loc` views and
  the report location, the CLIF reads' twins. Also the views no CLIF reads yet (tape, `ast_depth`, `node_order`,
  `next_sibling`), their first definition here.
- `rerail_tape_format::reads::f16z`: 16z's reads through the shared cache (`cache::get`, the CLIF lookup's twin,
  decision 0536's shape), so the host's reads and the device's decode into the same cache.

The directory's reads layer is written a row at a time (`made::Docs::reads`), on its document's first read through
the views, so a lazy batch makes only the documents it reads (decision 0494). Its homes are fixed: row `i`'s tape in
home `2i`, its lines in `2i + 1`, the 16z cache in `2n` (`ViewDir::cache` names it once made). A row holds its tape's
length (`ViewRow::bytes`, in the row's tail padding: the layout the kernels read is unchanged). The device's
directory (`device_chunk::views_in_place`) is the batch's, now held by the batch rather than its chunk: it writes
every row through the same `reads`, then finds what the device reads. A layered source's directory (`views_over`)
is still made whole, apart.

`Viewed`'s point reads (`value_memo`, `row_at_memo`, and `value` and `row_at` through them) find the document by the
memo and read the column through its row (`Docs::read`, a `Doc`). A row is read off the projection only where it
has no tape read, counted once by why (`views.refused.*`): a document in another block, a tape not read in place,
a 16z entry larger than the batch's cache was made for (a lazy batch's cache is sized by its first 16z document),
and, for the `Loc` views, positions not in the block (a guest's segments or mapped placement). `Viewed`'s scans
(`each_row`) and lookups still walk `Projected`; they go next, with the host structures of step 4.

Tests: `the_views_read_through_the_directory_are_the_projections` (every view's every row and column, both ways,
through `Source::value` and `row_at` too, each format, core.rerail's shape and T's, a docstring-only document among
them), `the_view_reads_are_the_hosts_rows` (the Rust reads against the JIT's and the hand-made rows),
`the_rust_16z_reads_are_the_jits`, and the syntax tests (71 on Linux).

Cold hogql gives 1,629 findings at 91.3 B and 91.3 B instructions (91.4 B and 90.0 B before; this Mac at a load of
about 12), and its profile counts no `views.refused.*`: every document's rows are read through the directory. On
Metal, the M3 passes `the_kernels_directory_searches_are_the_hosts`, `a_views_index_is_in_its_block_for_every_thread`,
`views_made_on_the_device_are_the_hosts_rows` and `a_view_made_on_the_device_is_made_at_each_stratum` (the mini's
Xcode license is still not agreed to).

**Step 3, the scans (2026-11-21).** `Viewed`'s scans (`each_row`: `for_rows`, `for_rows_in`, `for_rows_keyed`,
`rows_of`, `filter_eq`) and its lookups' matches (`read_matches`) read each row through its document's directory
row (`Docs::read`, a `Doc`), as its point reads do: a row is its view's read of the entry and the tape, the
kernels' reads' twins. The walk over the projection (`each_row_projected`) is left for the rows with no tape read
(`views.refused.*`, counted). A lookup still finds its rows by the projection's rank index and granule table (in the
batch's block): only the columns are read through the directory. `documents_of` reads the keys from the directory
and the file from the document's input, so no document is made for them.

**Step 4, the host structures (2026-11-21).** Each one of the table goes with its last reader, or moves into the
batch's block:
- `made::Docs::tags`: a document's tag is its tape's (`Facts::tag`, read from its input).
- `made::Docs::given`: each document's counts as the documents' table gave them are written once beside the
  directory, in the batch's block (`made::directory`, `[nodes, fielded, siblings, places, 1]` a row, zeros where
  none was given).
- `Projected`'s `key`, `file` and `tag`: a projection's entry (`ViewDoc`) is made in its block as the projection is,
  with its key, file and tag; `Projected::key` and `file` read it, and `tag` is its tape's. The views' reads fill in
  the rest of the same entry on the first read (`ViewEntry::new`), once: the layered directory (`views_over`) names
  the same entry, where it made a second one before.
- `of_tag`, `programs`: the batch's documents by tag (`by_tag`, `tag_at`) and its programs by file are `Cells` in
  the batch's block.
- `kinds` (`KindRows`): the `node` view's index by kind is `Cells` in the batch's block (its keys, starts and
  matches), sorted in scratch where it is made.
- `given_rows`: the relations' kept counts are a table in the batch's block, one `[hash, rows]` a relation, by its
  name's hash, ascending, read by bisection (the families' relations and the text tests', by name since the hub's
  Q332: two names of one hash are both not given, so a count is never another's).

- `Shared` (`strings`, `at`, `atoms`): the batch's strings are atoms of the atoms directory (`Atoms::intern`,
  `value`), interned as the rows are made; `Shared` keeps only the batch's atom ids and their meets, and the
  admitted texts are read back from the directory (`texts_of`).
- The layered directory (`views_over`): made in the chunk's block from each batch's own reads layer
  (`views_in_place`), its rows and homes renumbered (a derived row each, `// copy: derived`); it names each batch's
  entries and keeps each batch's caches (`caches`), so no projection is read again for it. A batch not in the
  chunk's block is refused (`views.refused.block`, counted).

`Batch::ensure` now claims the documents it makes under the same lock that finds them missing: two readers side by
side could both find a document missing and both make it (`reads_side_by_side_make_each_document_once`, 1 run in 3
before; 10 of 10 after).

Left of the table: `made::Docs::slots` (each document's input and its projection, made on its first read).

Cold hogql gives 1,629 findings at 90.2 B and 89.9 B instructions (91.3 B and 91.3 B before; this Mac at a load of
about 12; 90.8 B and 90.5 B with `Shared` as atoms), and its profile counts no `views.refused.*`: every scan's
rows are read through the directory. On Metal, the M3 passes `the_kernels_directory_searches_are_the_hosts`, `a_views_index_is_in_its_block_for_every_thread`,
`views_made_on_the_device_are_the_hosts_rows`, `a_view_made_on_the_device_is_made_at_each_stratum` and
`cpu_and_metal_read_the_same_tape_values` (with the layered directory over the batches' reads).

**Step 3 on the CPU, row 7: a node's text is read, not made a row a node** (2026-11-22). In core.rerail's shape,
`core_rr` made `text` a row per node, eagerly, for every rule that read it. Now `text(v, t)` is the read
`t = @rr_node_text_any(v)` (`preparation::text::TextByNode`), through the run's binding of `rr_long_text_node`
(`LongTexts`, which holds the batch and reads the node's bytes in its tape), wherever `v` is a node:
- bound in a column declared `node` of a base relation (`rr::core_node_columns`, read off core.rerail's text, since a
  version lowers `node`, `span` and `loc` alike), or of a matcher's relation;
- or bound in a column of a file-local derived relation that only ever holds such nodes (a greatest fixpoint over the
  rules' heads, `node_columns`, over `FileLocal`'s split).

`text` stays a relation, read as rows, where its node is a span, is bound by the text itself (`text(n, "x")` binding
`n`), or is bound only by a cross-file relation. That last case matters: the cross part runs over versions, and a
warm retraction would read the node in the edited file's new document, since keys are per path. A pass that reads
a node now counts as reading its input (`file_local::reads_a_node`: `InputsRead`, `crossing`), and the long-text
binding is kept with its documents even with no rows (`Values::add`, `Layered::sources_of`).

Two warm-edit costs found on the way and removed, with no new structure:
- The 0090 §4 threshold sized a changed read by its source's `len()`. For a corpus batch's made-on-read relation
  (`MadeRows`) that made every document to count it, the unchanged ones too
  (`a_warm_edit_of_a_host_file_reads_as_a_cold_run`). It now reads the rows the run was planned with
  (`model.stats.card`): for a semijoin, those at the nodes the exports hold, which is all its strata read.
- A semijoin's kept statistics are patched by an edit at the held nodes (`held_counted`): the changed documents'
  old rows at the nodes held before, out; their new rows at the nodes held now, in. They are not counted again over
  every held document (`r4_a_base_relations_statistics_are_kept_and_patched_by_an_edit`).

`warm_tests`' `HOSTED` binds its `text` node cross-file now (`used(c) :- called(c, x), defined(x)`), so the
warm-edit tests keep covering the relation route.

Tests: `text_of_a_bound_node_is_read_in_place` (each kind of binder; spans, lookups by text, cross-file binders and
T's shape kept), the warm and made-batch tests, and the quick tiers of rerail-logic, rerail-tests-logic and
rerail-tests-lang on Linux. Cold hogql gives the same 1,629 findings at 80.1 B instructions (88 B before; this Mac at
a load of about 11), and row 7 counts 0.0 MB and 0.3 ms.

**Step 4: the GPU reads every view in place** (2026-11-22). The chunk is gone: `device_chunk::build`, its node
columns and codes (row 3), `expand_view` and the views made from them (row 4), and the line starts and spans copied
beside them (row 14). `chunk_views.uglykl` is deleted. A view the batch's directory does not read in place is not
made on the device: `Source::device_view` gives `None` (counted as `views.refused.not_on_device.<view>`), and the
stratum reads the source's rows where they lie, or uploads them as any other host relation (row 5, step 6).
- **The kind-set axes are solved off the tape.** `axis_count` and `axis_write` (join_stages.uglykl) climb each node's
  ancestors through `clif_view_up` over the batch's `node` view (`ViewsInPlace::node_view`), and test each one's
  kind atom against the set's bits. They write the pairs' ids into one relation in a block (`Part::over_key_runs`),
  with each node's run start in the batch's block (`metal_join::axis_in_place`, installed as
  `nearest::AxisSolve`). A layered source's axis is solved the same way over its layered directory
  (`views_over`), the batch's rows and then each value's. An axis is no node source, so the layered solve takes the
  axis's own batch (`Within::batch`); asking `python_batch` sent containment's every-ancestor relation to the host,
  873 MB uploaded a run.
- **A view filtered by constants is read in place whole** (`WHERE idx = 0`, `View::eqs`; row 5's step-4 share). The
  stage adds each constant as a key (`with_consts`): its index is over its key columns and the filtered ones, a
  parent's walk (`by_parent`) takes the children off the tape, and `matches` checks every candidate against the
  constants. Its columns are the source's (`View::column`), never `owner_col`'s, which maps a whole view's alone.
  A stage that checks no keys (a filter, mode 3) uploads the filtered rows as before.

Measured on cold hogql on the M1 Max, Metal: 1,629 findings, the same digest as the CPU run. The copies table counts
35.9 MB in all, 662 MB with only the chunk removed (the axis uploaded), and 69.7 MB of row 5 before. What is left is
row 5's 39 calls of host relations (step 6), rows 8 and 9 (step 6), and row 17 (step 4a). Tests:
`a_filtered_view_is_read_in_place_its_constants_keys` (a view of reordered columns, each kind of probe and a
filter's), `a_has_axis_on_the_device_is_walked_on_the_tape`, `views_read_on_the_device_are_the_hosts_rows`,
`a_view_not_read_in_place_is_not_made_on_the_device`, and the containment test over `axis_in_place`. They run with
the rest of rerail-device-metal's setfix and join tests on the M1 Max, and the syntax tests run on Linux. The
device-metal test binary now registers the carried context and the core names as it loads (`test_load`), as
rerail-logic's does. Without them, five tests whose runs hand jobs to the pool panicked.

**Step 6, row 5: nothing is uploaded** (2026-11-22). `DevRel::upload` is deleted, with the reports' upload
counts. A relation the device binds lies in a block, or is a view read in place. One of no rows is an empty relation
of the device's own. Any other refuses its stratum at run time, named
(`device.not_in_place (why): what`, "has no device form"), and the stratum runs on the CPU, which reads the rows where
they lie. On cold hogql the last 39 uploads were relations of no rows, so no stratum is refused for it. The tests
that took an upload now expect the refusal: a view whose tapes the device does not read, a filter's filtered view,
and `has` read whole.

**Step 6, row 9 on hogql: an axis has room for every key.** Its last two copies were the axis relation
(`Part::over_key_runs`), made with room for two indexes, `n` and `a`, and probed by three keys: `!within(n, a)`
probes both. Its two columns have at most four keys, so it now has room for four (`Part::AXIS_INDEXES`). The room is
address space, its pages taken as an index is written. `DevIndex::moved` stays as the counted fallback of a relation
in a shared block of the run's that has no room left. On the M3's 300-file run that was most of the row (ab110,
ab111). It goes when such a relation is made with its readers' index room, measured there.

Cold hogql on the M1 Max, Metal: 1,629 findings, the CPU run's digest, and 27.5 MB copied in all: rows 8 and 17.

**Step 6, row 8: the CPU's head rows are written where they lie** (2026-11-22). A wave's writers write a head's
rows into its wave room (`rows::WaveRoom`, `Rel::wave_room`), past the relation's rows. The room is the tail's own
room (lent, as the device's is; or the tail moved where its block has no room, as a push moves it). Past the
tail's block it goes on in regions of their own, each with room for twice the rows before it. A writer claims 1,024
slots at a time under the room's lock, which also makes the room as claims pass what is reserved. A row is written
once, into a slot that is already its relation's. `Fresh` holds its spans of slots, not rows.

- **Settle, in place.** The settle sorts slot ids, drops repeats and held rows, and lays the kept rows out in place
  (`rows::permute`). Each row moves once along its permutation cycle, with one row held aside for each cycle.
- **Morsels.** A morsel's rows are absorbed as they lie: `absorb` appends its spans, and the rest of the last span
  before them is a hole. The wave's end lays the kept rows out at slots `0..m` (`WaveRoom::lay_out`). `finish` then
  claims them in the tail and appends the regions past it as the relation's tail.
- **Internal space.** What the writers left between rows (a span's unwritten rest, a dropped repeat) ends up past
  the rows. It is room the next wave writes into, not space inside the relation.
- **Determinism.** The order of the writers' claims does not reach the relation: the kept rows are laid out in the
  first order, on every column, the same order the old merge gave.
- **The device's rows.** The device writes a head's rows into the same room (`Placed::Wave`), giving back the slots
  past the rows it wrote while they are the last claimed. On its own, the device's lots lie at slots `0..m` and are
  claimed without a host read. Beside the CPU's rows, a CPU row that a device lot repeats is dropped (the lots are
  read where they lie), so no device row is read back into `Fresh`. The rows read back through `add` are written
  into the room too.
- **A lattice.** Its candidate rows go into a room that is a relation of its own (rule heads in a block), and the
  fold reads them there.
- **The closure seed's rows** are taken the same way, pending as a push puts them. A first build gave a leveled
  closure a row twice. The device's leveled-recursion test caught it, comparing against the CPU's rows.

Tests: a wave's four writers, past a block, laid out in place (`a_waves_writers_write_in_place_and_their_rows_are_laid_out`);
`permute`'s cycles (`rows_are_laid_out_by_their_cycles`); the existing `Fresh` tests (merged not sorted, checked
once as a sorted batch, a reorder), the scalar tests, and the device tests.

Cold hogql on the M1 Max, Metal: 1,629 findings, the CPU run's digest, and 6.6 MB copied in all: row 17 (step 4a).

The extension's fixpoint (`logic/native/extension/setfix.rs`) is a second engine whose relations are vectors, not
blocks. Its `Fresh` is not this row's.

**Step 6, row 9: nothing is copied for a key** (2026-11-22). `DevIndex::moved` is deleted, and so is the report's
`moved` count. A relation whose block has no room for a key's index now refuses its stratum, which runs on the CPU
reading the relation where it lies, as row 5's uploads do. `device.no_index_room` counts each refusal, by why the
block refused (its directory full, or no bytes left) and by where the relation lies. A view's relation of no values
whose block is full is still made anew (`DevIndex::fresh`): it holds no rows.

- **Room for the directory.** A relation alone in a host block (`Part::with_index_room`, `over_key_runs`: a kept
  source, an axis) now has room for an index at every entry of its directory (`ADDED_INDEXES`, 16). Before, each
  maker guessed a count: its columns and two, its columns and one, or four. The room is the block's address space,
  and its pages are taken only as an index is written. `Part::AXIS_INDEXES` is deleted.
- **Measuring it.** Run with `--metal`, the 300-file bench now binds the device (before it, the bench's strata ran
  on the CPU and the earlier "0 moved" measured nothing). Two kinds of read were added so the device reads more in
  place:
  - a filtered view of a source in a block, whose constants are keys the stage adds;
  - a derived relation's rows, which `Derived` now gives as its relation's block or its whole view's.

  One run copied 8.9 MB in 8 relations (`DevIndex::moved`). The next run of the same build copied none, since the
  copies depend on which strata the device takes.
- **A head's later lots.** Where an earlier lot's block has no room past it, a head's next lot goes on in the wave's
  room (row 8), not in a relation moved to a block with room (`Part::moved_room`). The device's large head lots no
  longer get a block of their own (`ALONE_ROWS` is deleted): they are written into the wave room like the CPU's.

The 300-file bench on the M1 Max (`--metal`, load 8–10): 463 violations, as before. Copies 52.9 MB: row 7
(step 3) 49.9 MB and row 17 3.0 MB. No row 9 copies, no index refusals, and no relations merged. Cold hogql: 1,629
findings with the CPU run's digest, and 6.5 MB copied, all row 17.

Test: `a_key_past_a_relations_room_is_refused_not_copied` replaces `a_relation_moved_for_an_index_lives_until_its_batch_is_done`.

Left in row 9: `Rows::merged`, and a chain's compaction past `MAX_SEGMENTS`.

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
