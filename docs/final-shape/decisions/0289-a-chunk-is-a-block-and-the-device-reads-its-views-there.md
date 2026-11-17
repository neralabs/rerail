# 0289: A chunk is a block, and the device reads its views there

Status: draft (2026-11-15), being built. §3's second half was overtaken by 0293 step 4 (2026-11-16): the device's
reads of the tape wait for 0293 step 5. 0266's path step 2 (`Chunk`), taken here at the user's choice. It is
done in the order that removes the measured cost first. Builds on 0266 (blocks), 0254 (chunks evaluated at once)
and 0277 (what the device reads).

## Intro

A stratum on the GPU reads the relations its rules join. A relation the store holds (a flat relation) is copied
into a device buffer with its columns laid end to end. A view is a relation over a batch's documents (`node`,
`child`, `field`, `tape`, `core_node`, `node_order`, `line`). For a view, each row was made on the host, one at a
time: `Source::row_at` finds the row's document again, then computes each column from the document's arrays.

On hogql with one rule, on Metal (`2b2ca1b00`, which splits the uploads):

| Upload | Bytes | Host CPU |
|---|---|---|
| Flat relations | 22.9 MB | 4.4 ms |
| Views | 20.2 MB | 22.4 ms |
| Names | 0 | 0 |

Views are 84% of the upload CPU, for less than half the bytes. Each view value is arithmetic over arrays the batch
already holds:

| View | Columns | From |
|---|---|---|
| `node` | document, node, kind, start, end | the document's key; the node's id (`Projected::global`); `Facts::kind` through the dialect; `Facts::start`, `Facts::end` |
| `child` | parent, index, child | the parent's id; the child's index among its siblings (`Projected::idx`); the child's id |
| `field` | parent, field, child | the parent's id; `field_atom(tag, Facts::field)`; the child's id, over `Projected::fielded` |
| `core_node` | document, tape node, node | the key; `Projected::core`; the node's id |
| `tape`, `node_order`, `ast_depth`, `line` | parent, size, depth, order, first line | `Projected`'s lazy arrays |

0266 puts a chunk's documents in one block, and everything made of them with them (its §1 `Chunk`). Once the arrays
are there, the device can compute a view's columns itself, and nothing is made on the host.

## Decision

### 1. A batch's chunk block

When `syntax::Batch::build` makes a wave's batch, it also makes the batch's `Chunk` block (a `rerail-blocks`
block, leased from the GPU pool so that a dispatch never maps it again, 0266 §15). For each document, the block
holds:

- its key, tag and node count, and its row ranges in each view (`Batch::nodes`, `children`, `fields`);
- the projection's arrays, by local id:
  - `core`, the tape node of each node;
  - its id's low 32 bits (`DefIds::low`, or the preorder index);
  - its parent's local id (`Projected::up`, made in the walk: the tape stores no parent, 0293 §6);
  - its index among its siblings;
  - the fielded nodes;
- what the views read of each node, by local id: `start` and `end`, `kind` (the atom the dialect gives it) and
  `field` (its atom). These were the facts' arrays, by tape node; since 0293 step 4 `Facts` holds no column, and they
  are the tape's answers, read through its API when the chunk is made (§3).

The document table is a `structure!`, so the same layout is the host's Rust struct and the kernel's MSL struct
(0266 §4). There is no hand-written copy.

### 2. Views made on the device

A stratum whose rules read a view no longer uploads it. A kernel (`VIEW_EXPAND`) computes the columns the stages
read straight into the device relation the stages already read: one thread per row, finding its document by
binary search over the row ranges. That relation stays resident for the chunk's other strata, as uploads do now
(0277). Nothing downstream changes: probes, sorts and stages read it as before.

A view the device cannot make (a lazy family's `py.*` rows, `has`, `tape`, `node_order`, a guest's places in its host
file) is uploaded as before, and named in the profile's split.

Check: the device's parity tests, with a view relation in each shape. Measure: the light run's view uploads, which
should go from 22.4 ms of host CPU to the block's build (a copy of each array, once per batch) plus the kernel.

Built for `node`, `child`, `field` and `core_node` (`device_chunk.rs`, `metal_join`'s `view_expand`). The check is
`views_made_on_the_device_are_the_hosts_rows`: every column of each view, and a subset out of order, against
`Source::value`, over documents with fields, nesting and a broken one.

Measured, cold on hogql (the same 1,629 findings):

- Views made on the device cost about 0.3 ms of host CPU per MB (6.7 MB in 2.0 ms). Views made on the host cost
  about 1.3 ms per MB.
- What is still made on the host is mostly `Within`'s axes (`rr_nearest_*`, `rr_inside_*`, `rr_has_*`): 17 uploads,
  18.4 MB of the 21.7 MB. Making their rows is a climb of each node's ancestors, so their 39 ms is the climb, not
  the copy.
- They are next. Every array the climb reads is already in the chunk: `parent`, `local` and the kind. An axis's set
  is a kind bitmap. `Inside` and `Has` have several rows per node, so a count pass and a scan come before the
  scatter. A relation on the device is a set: no reader depends on the host's row ids.

Then built for the axes the batch's solve answers (`nearest`, `inside`, `within`). The solve's pairs of node rows are
copied into the chunk (`put_pairs`, 8 bytes a row, once per axis), and `view_expand` makes each pair's two ids
(`VIEW_AXIS`). The per-value search of `Batch::id` goes. `has` is made per looked-up node and is uploaded as
before. The check is the same test, which now also covers the three axes and checks that `has` stays on the host.

Measured cold, with the same placement (7 strata) and the same findings:

| | Uploads' host CPU | Views on the host | Views made on the device |
|---|---|---|---|
| Before | 48.2 ms | 30.2 MB in 39.8 ms | 6.7 MB in 2.0 ms |
| After | 30.1 ms | 6.9 MB in 2.6 ms | 21.7 MB in 22.4 ms |

Of the 22.4 ms, 14.4 ms is the axis solve, on the host and on Metal (`kind-set axis solved for the batch`), and
5.3 ms is the chunk block's build (0.3 ms a batch). The copy and the kernel are the remaining ~2.7 ms.

Then the solve itself, on the device's binding (`Binding::Metal`), made in the chunk
(`metal_join::axis_in_chunk`, registered as `facts::nearest::install_chunk_axis`). For each node row, `axis_count`
climbs the node's ancestors through the chunk's arrays, checking each against the axis's bitmap of kinds (256 tags
× 1,024 codes). The climb is bounded by the document's nodes. A scan of the counts gives each node's first row.
Then `axis_write` writes the pairs, nearest first, in the host's order. The starts and the pairs are page-aligned
in the chunk and bound with no copy, and the host reads them there in place (`Within::starts`, `pairs`).
The count, the scan and the write are one submission: the pairs' space is sized before the count. A nearest
axis has at most a row per node, so its bound is exact. Containment's is 8 rows a node; a batch past it skips the
write, and the host writes again into space of the total's size (the profile names it). The space past the total
is given back to the block (`Block::shrink_last`), and pages the device never wrote are never memory. The host's gather of every
node's parent and membership goes, and so does the copy of the pairs. Containment (`within`) is built with the
CPU's binding: it keeps the host's solve, and its pairs are copied. When the chunk cannot hold an axis, the solve
falls back to the host's, and the profile says why.

Checks: the device test compares the device's solve with the host's: rows in order, both columns, and each
looked-up node's run. The engine's `the_kind_set_axes_hold_exactly_their_pairs` runs `Binding::Metal` against its
oracle.

| Cold, hogql, same findings | Uploads' host CPU | Views on the host | Views made on the device |
|---|---|---|---|
| Before §2 | 48.2 ms | 30.2 MB in 39.8 ms | 6.7 MB in 2.0 ms |
| Axes' pairs copied | 30.1 ms | 6.9 MB in 2.6 ms | 21.7 MB in 22.4 ms |
| Axes solved in the chunk | 18.2 ms | 1.3 MB in 0.3 ms | 19.9 MB in 11.5 ms |
| In one submission | 13.9 ms | 4.7 MB in 1.5 ms | 21.9 MB in 7.3 ms |

No axis is solved on the host now: 18 are solved in the chunk. In the last run, of the 7.3 ms, 5 ms is the chunk
block's build (about 0.3 ms a batch), which §3 removes, and the axes' solve is about 3 ms of CPU for 18 (5 ms before; its wall time went from 11 ms to 6 ms). No batch went
past containment's bound. Placement varied between these runs (4 to 8 strata with a run on the GPU; the last run
put more on the GPU, so more views were read), so the uploads are compared, not the totals. The check of the
second write is the device test, with a bound of no rows.

Then `core.rr`'s places (`View::Loc`: `line`, `end_line`, `col`, `docof`). With placement repriced (below), the
strata it moved read them, and they were most of what was still made on the host: 1.05 M values of `line` in 13.4
ms, about 13 ns a value (a document found again, then two searches of its lines, per row). On a `Loc` view's first
device read (`device_chunk::fill_locs`, once per chunk), each entry gets its first row in the `Loc` views, its
spans past its nodes (`[start, end]` each, as `Projected::span` gives them: tokens, the whole text, lines and end
points) and its line starts (`core_rr::Lines`). `view_expand` finds the row's document and span, then its line,
last line and column by searching the line starts, as `Lines::place` does. A span's id past the ids by definition
is `granules::span_low`'s, in MSL, with the same constants. A guest placed in its host file keeps the host's view.
The check is the device test, over a `core.rr`-shaped batch: every column of the four views against the host's.

Measured cold, at the repriced placement (10 strata with a run on the GPU), the same findings: views made on the
host went from 22.7 MB in 23.0 ms to 9.7 MB in 5.7 ms.

**An ERROR node is a member when its atom is asked** (2026-11-16, at the hub's merge of 0293). The host's bits
left out slot 0, after `Within::member`, which read a node's code by `kind_code`: ERROR's is `u16::MAX`, past the
1024-slot table, so an ERROR node was in no set. ERROR's atom is slot 0 (`kind_atom`), as the kernel reads it
(`atom % 1024`), as the tape's set reads it (0293) and as the `node` view reports it. Both now read the slot. Found
by `views_made_on_the_device_are_the_hosts_rows`: on the CPU binding the tape's solve (0293 step 3) gave an ERROR
parent as the nearest member, which the device skipped.

### Placement repriced

Placement priced a value the device reads at 7 ns (`DEVICE_UPLOAD_PER_VALUE`), measured when every view's rows were
made on the host. From the uploads' split, it is now about 2.2 ns: a flat relation's value 1.7 ns, a view's made on
the host 2.5 ns, a view's made on the device 2.7 ns (its share of the block's build, of the axes' solve and of the
kernel). At 2.2 ns, more strata go to the GPU: 10 to 13 with a run there, against 4 to 8. An A/B of the two prices,
each run cold three times back to back, with the same findings: 72.7 G instructions retired at 7 ns, and 71.7 G
at 2.2 ns, fewer in every pair (user CPU 6.31 s and 5.88 s; the machine was loaded, so wall times are not compared).

### 3. The host reads the block

`Viewed`'s rows (`value_at`) read the same arrays in the block, so a view has one definition. The projection's and
the facts' `Vec`s move into the block, and the copy of §1 goes. This touches `Facts`, which a memo hit and a cache
share (`Arc<Facts>`). It waits until a memo entry is a document subtree that a hit copies into the chunk
(0266 §1, `chunk::add_hit`).

Built in two halves, as their storage allows.

**The projection's arrays are made in the batch's block.** A batch (`Batch::build`) opens its block before it projects
its documents, and each projection (`Projected`) writes its arrays there as it makes them: `core` and `local` in the
walk, `low` in the granules' numbering (`granules::number` writes into the block), the ordinals and the fielded nodes
on first read. The host reads them as slices (`rerail_blocks::Cells`, a run of a shared block). The chunk is nested
in the same block: its entries point at those arrays (`device_chunk::point`), and their copy goes. A projection made
apart from a batch (`Projected::new`: one document's numbering, its definitions) has a block of its own. A batch's
block is made whether or not the device reads it: it reserves address space, and holds only what was written.

**The facts' arrays are gone (0293 step 4).** This half was to move `Facts`' `parent`, `start`, `end`, `kind` and
`field` into the block once a memo entry was a document range a hit copies into the chunk (0266's path, steps 4 and 5).
Since 0293 step 4 `Facts` holds no column: a node's kind, flags, field, span and parent are the tape's answers,
through one API over every format, and the tape image is already in the chunk (§4). So nothing of `Facts` moves.
What is left:

- **The parents are the projection's.** `Projected::up`, the parent of each local id found in the walk that numbers
  them (0293 §6: a pass over them costs 1.2 M instructions, against 54 to 120 M read off the tape), is made in the
  batch's block with `core` and `local`, and the chunk points at it. The device's climbs (`parent_of`, the axes'
  solve) read it by local id, and `local` (by tape node) leaves the chunk.
- **The device reads the tape in the chunk** (§2's views and the axes' solve), with the kernels 0293 step 5 brings
  (the lab's format-16 kernels, E30 and E31), when they land; a format-15-only reader written now would be thrown
  away. Until then, the chunk holds what the kernels read of each projected node (`start`, `end`, `kind`, `field`,
  by local id), read off the tape through its API once a batch on the device's first read: an interim, as 0293's
  own (the matcher's records), and nothing is measured before step 5.

Found at the merge (Native-and-GPU-strata, d8fd02e57): the tape's code set of an axis (`Within::set`, 0293 step 3)
counted an ERROR node as a member when the set names ERROR, where `Within::member` and the device's solve did not.
That merge left ERROR out of the set; the hub, meeting the same mismatch, had made the host and the device count
it ("An ERROR node is a member when its atom is asked", above). The user chose the hub's form (2026-11-16): the set
reads an entry flagged `CODE_ERROR` as slot 0, a member where the set names ERROR, and
`the_kind_set_axes_hold_exactly_their_pairs` holds a document with ERROR nodes and a set naming ERROR and asserts an
ERROR node is a member (work queue Q38).

Checks: `views_made_on_the_device_are_the_hosts_rows` (every view's columns, read through the arrays in place);
`definition_ids_are_pinned` (every id as it was); the projection's tests (`facts::vtab::syntax::tests`); the axes'
oracle; the matcher wave on Metal.

Measured on hogql, an A/B of the two builds, each run cold twice, alternating, the same 1,629 findings. The load was
about 30, so time is CPU and instructions retired, not wall:

| | Instructions retired | User, system CPU | Chunk block's build | Projection (`projected`) |
|---|---|---|---|---|
| Before, run 1 | 90.7 G | 6.61 s, 1.87 s | 4 ms, 10 batches | 98 ms |
| §3, run 1 | 78.3 G | 6.22 s, 1.19 s | 2 ms, 10 batches | 110 ms |
| Before, run 2 | 89.7 G | 6.40 s, 1.74 s | none built | 105 ms |
| §3, run 2 | 76.3 G | 6.29 s, 1.14 s | 3 ms, 14 batches | 103 ms |

The chunk's build lost the projection's copy (about 1 ms of its 3 to 4). The projection itself is within the runs'
spread; its first writes into a fresh block are page faults where a `Vec` reused freed memory (`identity maps` and
`projection fields`, a few ms higher). The 12 to 13 G fewer instructions are not §3's: a later cold run of the §3
build took 90.7 G, as the runs before it did. They are the spread of a cold run (placement put 8 to 12 strata on
the GPU in these runs), and §3 is not measurable in the whole run's instructions.

No device reads the arrays on targets without Metal: there `cells` is a heap array with the same
API (no address space to reserve on WASM). `rerail-accel`'s dependency on `rerail-blocks` is Metal's only (its
`gpu` module is), so the WASM build no longer compiles `rerail-blocks`, whose blocks are `mmap`s.

### 4. Tapes and sources in the chunk

The residency's slabs become chunk blocks:

- a document's tape image, source, parents, codes, line starts, child lists and nodes by code are nested in its
  entry;
- `DocView`, its MSL copy and `Slabs { s0..s3 }` go: a kernel binds the chunk block and reads its document table;
- across runs, a document hit copies its subtree into the next run's chunk, instead of keeping a slab resident.

The Metal tape tests (`metal_tape_tests`, `tapes_are_tape`) are the check.

Built in two steps, each leaving the engine working.

**Step A: the slabs are blocks, and a batch reads one.** The tapes reach the device from the AST matcher wave's label
passes (0106, 0107), which admit a batch's documents into the residency (`admit_docs`) and bind a table over them.
Before, a slab was a raw lease of the pool, and a batch bound up to four of them (`MAX_SLABS`). A batch over more was
refused and split.

- A slab is now a chunk block over a lease of the pool (`lease_block_within`, 0266 §16): its rooms are the block's
  allocations, zeroed by the block when the lease is recycled.
- A resident document records its range in its block (`Resident::at`, `len`): every part of it is inside.
- A batch's documents are gathered into one block (`slabs::gather`, `DocTable::one_block`). Those not in it already
  are copied there, a range each, their view's offsets moved with them. The residency then finds them there, so a
  later batch over them copies nothing. The newest block takes the copies when they fit, else a new one holds them
  all. Kernels bind that one block (the four `Slabs` slots all point at it), and no batch is split for its slabs.
- A document admitted from memory (`Parts::Bytes`) is no longer checked against its SHA-256 twice: its digest was
  made from those same bytes (`TapeDoc::tape_digest`). One read from the store (`Parts::Frames`) still is.

Checks: `a_batch_over_several_slabs_is_gathered_into_one_block` (documents in two blocks gathered into one, every
part read as before, and asked again, nothing copied); `stored_tapes_are_admitted_into_slabs_and_read_in_place`
(no longer ignored: it is hosted in the engine, where the languages are registered); the device's tape test; the
metal crate's recycled-slab test; the matcher wave on Metal (`the_wave_finds_ws_matches_on_metal`,
`the_wave_is_one_evaluation_on_metal`).

Measured cold on hogql, the same findings. The label passes ran over fewer documents in the second run (the wave's
lane varies), so per document and per byte:

| | Batches | Documents | `residency.docs` | Admission |
|---|---|---|---|---|
| Before | 22 | 653 | 44.4 ms, 68 µs a document | 32.2 ms for 18.1 MB, 1.78 µs a KB |
| Step A | 17 | 434 | 20.3 ms, 47 µs a document | 13.1 ms for 11.9 MB, 1.10 µs a KB |

The machine's load was lower in the second run. What is left of an admission is the image serialized twice (once for
its digest, once to copy) and the derived parts.

**Step B: `DocView` and `Slabs` go.** Kernels bind the batch's one block, and read each document through its entry
in the batch's document table.

- The entry is a `structure!` (`slabs::ChunkTape`), whose MSL is generated with its size and offset assertions
  (`slabs::msl`), in place of `DocView` and its hand-written MSL copy. The tape view's sources (the label passes', the
  pattern kernels', the resident interpreter's) are built once, with the generated entry first.
- The entry holds the document's first byte in the block (`at`), and each part as an offset from there. Copying a
  document into another block (`gather`) changes `at` alone. The entry is not made of spans: spans are relative to
  their own address, and the table is its own buffer, apart from the block.
- `Slabs { s0..s3 }` and `tape_slab` go: every tape accessor takes the block's pointer (`tape_doc` is `chunk + at`).
  A label kernel binds the block at one slot (27), not four; the resident interpreter reads it at its `nodes` slot.
- `DocTable` holds one block (`block`), and refuses documents that are not all in it. `MAX_SLABS` now bounds only a
  batch of sources (block programs, 0067).
- **Since the merge of 0293's device half and 0310 (2026-11-16):** the entry is `rerail_clif::view::TapeView`, the
  one view the reads written once in CLIF take (0310), with `at` as its first word (it was `slab`, a binding index).
  Its MSL is written from it (`view::msl`), every field's offset asserted, so the generated-entry rule above holds
  with that type in place of `ChunkTape`. Its parts are 0293's: format 15's columns or format 16's arrays, no
  parent, child-list or by-code arrays. Each part is still an offset from `at`, and the reads take `chunk + at`
  as their base (`TAPE_READ`).

Checks: the device's tape test (`the_device_reads_resident_tapes_as_the_packed_batch_records_them`, no longer
ignored: it is hosted in the engine, where the languages are registered), the residency's three slab tests and the
matcher wave on Metal, as in step A.

Measured cold on hogql: the same 1,629 findings, and nothing gathered (every batch's documents were in one block
already). Step B changes what kernels bind, not what admission does, so the costs stay where step A left them. The
lane gave the label passes fewer documents again, and fewer of them hits:

| | Batches | Documents (hits) | `residency.docs` | Admission |
|---|---|---|---|---|
| Step A | 17 | 434 (200) | 20.3 ms, 47 µs a document | 13.1 ms for 11.9 MB, 1.10 µs a KB |
| Step B | 10 | 308 (33) | 16.5 ms, 54 µs a document | 11.6 ms for 12.1 MB, 0.94 µs a KB |

Per document it is higher because more of them were misses. What is left of an admission is as in step A.

Then the image is serialized once. A document whose digest is not known yet (parsed in this run, not restored from
the store) serialized its image to hash it and again to copy it. `TapeDoc::tape_digest_and_image` now returns the
bytes it hashed, and admission copies those (`resident_parts` takes them). The hits' bytes are dropped unread.
Measured cold on hogql, the same findings:

| | Batches | Documents (hits) | `residency.docs` | Of which not admission |
|---|---|---|---|---|
| Step B | 10 | 308 (33) | 16.5 ms, 54 µs a document | 4.9 ms, 15.9 µs a document |
| Serialized once | 16 | 553 (191) | 20.1 ms, 36 µs a document | 7.0 ms, 12.7 µs a document |

Admission itself was 0.88 µs a KB (13.1 ms for 14.5 MB).

## Order, and why

§2 first: it removes the measured cost (the views' 22.4 ms), and needs only §1's copy of each array, made once per
batch. §3 and §4 then move the host's structures into the block, removing the copy and the slabs. Each step leaves
the engine working, and is measured by the profile's split of the uploads.

## Open

- A device stage reading a view in place, with no relation made from it: not before §2's relation is measured to
  cost anything. It is made once per chunk, and read by each stratum.
- The `py.*` families, which are rows made on first read: they are a chunk's file-local facts, and move with them
  (0266 path step 4).
