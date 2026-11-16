# 0289: A chunk is a block, and the device reads its views there

Status: draft (2026-11-15), being built. 0266's path step 2 (`Chunk`), taken here at the user's choice. It is
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
  - its parent's local id;
  - its index among its siblings;
  - the fielded nodes;
- the facts' arrays the views read, by tape node:
  - `start` and `end`;
  - `kind`, as the atom the dialect gives it;
  - `field`, as its atom.

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

### 4. Tapes and sources in the chunk

The residency's slabs become chunk blocks:

- a document's tape image, source, parents, codes, line starts, child lists and nodes by code are nested in its
  entry;
- `DocView`, its MSL copy and `Slabs { s0..s3 }` go: a kernel binds the chunk block and reads its document table;
- across runs, a document hit copies its subtree into the next run's chunk, instead of keeping a slab resident.

The Metal tape tests (`metal_tape_tests`, `tapes_are_tape`) are the check.

## Order, and why

§2 first: it removes the measured cost (the views' 22.4 ms), and needs only §1's copy of each array, made once per
batch. §3 and §4 then move the host's structures into the block, removing the copy and the slabs. Each step leaves
the engine working, and is measured by the profile's split of the uploads.

## Open

- A device stage reading a view in place, with no relation made from it: not before §2's relation is measured to
  cost anything. It is made once per chunk, and read by each stratum.
- The `py.*` families, which are rows made on first read: they are a chunk's file-local facts, and move with them
  (0266 path step 4).
