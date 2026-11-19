# 0402: Kept facts are packed chunks, appended to a chain of mapped files

Status: decided by the user (2026-11-18: "I would prefer a single format if possible … We certainly shouldn't be
storing 8-byte quantities … please go ahead and design and build this"). Replaces 0401 F1, F2 and F6's copy of a
run's relation as it lies (8-byte cells in doubling chunks, its hash indexes, a cumulative delta). Keeps 0401's
intent (F3–F5): the store's kept facts are blocks in mapped files, SQLite holds only which files, and readers
read them through a virtual table. Steps 1–7 built (2026-11-18).

## Intro

A run keeps the relations it derived, so that the next run after an edit derives only what changed. Two forms of
the same rows exist today:

- **Frames** (0104, 0295): each column as zigzag varints of the difference from the row above. Small (147 MB on
  ep2), but read only from the start: a process decodes a frame into a flat array of 8-byte values before it can
  use one row.
- **Kept blocks** (0401 step 6): the run's own relation copied whole. Readable in place, but 4.4× the frames'
  size (641 MB): every value takes 8 bytes, chunks double (up to half empty), the hash indexes hold 8-byte offsets,
  and each delta carries its predecessor's entries again.

This decision makes one form that is both small and readable in place. It is MLDB's tabular dataset's shape
(`plugins/tabular`, its int tables): rows are added to a writable chunk at full width; when the chunk is done it
is **frozen**, and freezing is when each column's encoding is chosen from its statistics; a frozen chunk is
immutable, compact, and any value of it is read in a few instructions without reading the values before it.

The pieces exist in `rerail-blocks` from 0266's spike, unused until now: `col::IntCol` (MLDB's
`RawMappedIntTable`: one descriptor, the encoding chosen by exact size) and `packed::Packed` (a relation's live
rows in canonical order, each column an `IntCol`, a key that leads the order found by binary search).

## What a value holds

Every engine column is an `i64` with a type code (`retain.rs`, `s` or `i`):

| values | bits that vary |
|---|---|
| atoms (`s`): a `u32` id | the ids a relation uses: often a few hundred distinct |
| node ids: `document << 32 \| slot << 18 \| index` | the document changes rarely down a column; slot and index are local |
| span ids: `document << 32 \| extra_low(kind, i)` | as node ids |
| integers: lines, counts, ordinals | a few to 20 bits |

None needs 8 bytes in a chunk, and none needs to be decoded from the chunk's start.

## Decision

### K1. One format: a kept relation is a `Packed`

A kept entry's rows are a `packed::Packed`: its live rows, in canonical order (the order key's columns, then the
rest), with no duplicates, each column an `IntCol`. When the run derived its rows' levels (decision 0090) they are
an `IntCol` of the same entry, in the rows' order. Dead rows are not kept (a frame never kept them either).

### K2. Encodings: random access, chosen by exact size

`IntCol` chooses the smallest of:

| tag | what | for |
|---|---|---|
| `CONST` | every value `base` | a part's document column, a constant |
| `PACK` | `v - base` in `bits` bits | atoms, small integers |
| `RUNS` | each run's value and first position | a sorted leading column with repeats |
| `LINE` | the residual from a line, in `bits` bits | a column that grows with its position |
| `GROUPS` (new) | per 128 values: their minimum and width, then each value less its group's minimum | sorted node ids, ids clustered by document |

`GROUPS` is the random-access counterpart of the frames' delta varints. A sorted id column's group of 128 spans
a small range, so its width is near the varint's bits; a group that crosses a document costs that group alone.
Value `i` is: group `i >> 7`'s base and its (width, bit offset) entry, then one packed read: a fixed number of
loads, no branch on the data.

### K3. Freezing is encoding, at the keep

While a run runs, its relations keep their writable form (8-byte cells in doubling chunks, 0266 §4): appends are
cheap and atomic there. At the keep, the store's writer packs each put:

1. each column read once, its atom ids mapped to the store's (a forked process's, 0170) as read;
2. one pass checks whether the rows are already in canonical order and distinct: if so, no sort. This is where a
   relation that declares its output sorted (K7) costs nothing, and an input's granule (its rows in node order)
   usually is;
3. else the rows are radix sorted on the packed key (`relation::sort_canonical`, freeze's), with their levels;
4. duplicates dropped, each column's encoding chosen and packed into the file, sized exactly beforehand.

`Packed` is the only frozen form a kept relation has. `Block::freeze` keeps its meaning (the file's header says
frozen, its length is final).

### K4. Appendable: a chain of frozen files

A space's kept facts are a chain of files, `<store>.facts/<space hex>/c<seq>-<tag>.blk`, oldest first. A commit
appends one file holding only what it puts: its entries, sorted by key, each a `Packed`; a gone entry for a
relation the space no longer keeps; a reads-only entry for a relation whose reads changed and rows did not (its
rows are the newest older entry's of the same key). No earlier file is rewritten, so the bytes a run writes are
its changed relations' packed bytes.

A reader looks a key up in the newest file first, then older ones (each a binary search of a sorted directory in
place). Each entry's `Packed` is one contiguous range of its file with relative references only, so it moves by
`memcpy`.

**Merging.** A write merges into its file the chain's newest files that weigh no more than twice what joins them
(its puts' expected bytes, at least 64 KB, plus each file taken), copying their live entries (each range copied, no
value re-encoded). So the files' sizes grow geometrically from newest to oldest: a chain of `B` bytes has about
`log2(B / 64 KB)` files, and a value is copied a logarithmic number of times (an LSM tree's tiered merge). When what
the chain holds that newer entries replaced outweighs what it still reads (and is over 1 MB), the write merges every
file. Files are made under a name no reader opens, frozen, renamed and their directory synced before the SQLite rows
name them; a file the committed chain no longer names is removed by the next write (atoms' A2, A5).

The first build (a roll-up of the whole chain every 8 files) rewrote ep2's 150 MB cross-document space about every
eighth commit; its chain had reached seq 1,010 after three runs.

### K5. SQLite holds metadata only

`facts_chain(space, seq, name, tag)`: which files a space's chain has, in order, replaced in the transaction that
commits the run's writes. No row data, no blobs, no per-relation rows: an entry's digest, reads and types are in
its file's directory.

### K6. Reading

`KeptRead` reads a chain as one: `get(key)`, `of(rel)`, `all()`, each entry giving its `Packed`, its levels and
what a run checks (digest, reads, types).

- **Step 5** binds a kept relation by reading its columns from the `Packed` into the run's relation (a column at
  a time, the atoms mapped to the process's ids), in place of decoding a frame. The frames' snapshot (read once a
  process, before the first wave) holds such a relation as a frame whose data is its place in the chain
  (`Data::Kept`: the space's mapped files, held while a frame names them, and the entry's key); retags, deltas and
  this process's own writes compose with it as with a decoded frame.
- **Which relations.** A relation is bound from the chain where its entry is its frame's: the same digest and
  types. Its deltas are not composed (the entry holds the rows after them), and its levels part is the entry's
  levels, in the entry's row order (the frame's levels are in the frame's). Any other relation (no entry, a stale
  one, a store from before the chain) is decoded from its frame until step 6 retires the frames. The first build
  marked a whole space by its first kept write; retain queues an input's frame before its entry, so the mark
  depended on the writer's order, and was replaced by this per-relation check.
- **Every frame write puts its entry.** A relation the cross-file run hands off (its rows dropped as the run goes,
  Q74) had a frame and no entry, and its older entry stayed (ep2: `p0_absent`, 58 kept rows against an empty
  frame; 1,095 frames of phase 0's space with no entry). Its entry is now queued at the hand-off, the rows lent to
  the writer until packed, and the part's write puts its reads. A relation not held in a block is put flat, with
  its levels.
- **Step 6a: the frames hold no rows.** A frame whose kept entry is put beside it (`Encoded::in_chain`, set by
  the writers that queue the entry: retain's and kept_strata's) is written as its relation's metadata only:
  digest, reads, types, row count, `seg = -1`, no bytes and no segment. Every read (the snapshot, the cross
  space, the SQL admission, a relation's parts) binds it from the chain; one with no entry of its digest is
  dropped, so its relation is absent and derived again: correct by construction, at the cost of a derivation.
  A delta frame keeps its bytes (its base's entry holds the rows after it, so a bound base reads none). A store
  from before keeps its frames' rows until they are written again. An input's granule frame and entry both have
  digest 0 (its validity is its documents' digests): the pair is queued together, so the entry is the frame's.
- **Rows read by position are kept as given.** An input's granule (its rows by owner: its readers' owner ranges
  are positions), its documents' table and its kept statistics (an encoded word stream) are put `given`
  (`packed::Order::AsGiven`): every row, in the order put, no order recorded. Sorted into canonical order, a
  granule's owner ranges broke (ep2's edit runs failed the owner check, and the empty source that stood in read
  past its rows) and the statistics' words were sorted and deduplicated. Every other entry records its canonical
  order on its first 8 columns (`Packed::okey`), so its leading column is a range (`Packed::leads`); before, the
  order was kept but not recorded, and no probe took the range.
- **A retag keeps its entry's digest.** A retag (new reads, the same rows) is put with no digest; its entry took
  that 0, and every retagged relation stopped matching its frame (ep2: 361 of the cross space's, so k1 derived 877
  of 1,146 strata again). The entry's digest and types are kept.
- **Step 6b: read in place.** A kept relation with no levels part, in a process that did not fork (its ids are the
  store's, decision 0170), is a view of its entry (`retain::kept_source`, `InRel::over_source`): no row copied.
  Its leading column is probed as a range of the entry (`sorted_range`); another column through its rows sorted
  by that column, made when first probed. A relation the run extends is made from its rows then (`Rel::based`
  takes no view; `Rel::compacted` records its start from the view's rows, so the maintain mode has its version).
  A relation with levels, or in a forked process, is read into the run's relation as before.
- **A source that could not make its rows** (a kept input's granule that does not decode) reads 0 within its row
  count, and the native stratum that read it fails with why (`Source::failed`, asked after each probe, never
  making rows to find out); before, the stand-in held no rows and a join read past them.
- **Step 6c** is the virtual table (K8).

### K7. Sorted outputs

A relation may declare that it keeps its rows sorted:

```
// `calls` keeps its rows in canonical order: by caller, then callee
.decl calls(caller: number, callee: number) sorted
```

Canonical order is the keep's (K3): the columns in turn, as signed integers. The declaration is a choice of
layout, not a promise the rules must keep: the engine makes it true.

- **When its component is done** (`InRel::moved`), its live rows are laid out in canonical order, once (one sort
  of their ids, dead rows left out, levels following their rows), and the run knows them sorted on its leading
  columns (`runs::Sorted`, decision 0180, over every row: on every column an order is the dedup order, which
  leads with a hash). An order a later stratum probes on a prefix of them is then the rows in place: no ids
  listed, none sorted, a probe a binary search of the rows.
- **The keep** finds them in order (K3 step 2) and sorts nothing.
- **The next run** reads them in place (step 6), the leading column a range of the entry.
- A relation maintained from a kept one keeps its rows where they are: its versions are read by row id (decision
  0364). It is laid out again when a run derives it whole.
- The engine is told by its component's spec (`sorted <rels>`), from the `.decl`'s qualifier (decision 0099's
  list, `front::QUALIFIERS`).

Not built: merge joins over two sorted relations, which the planner may choose once both sides are known sorted.

### K8. The kept facts in SQL

`kept_facts` is an eponymous virtual table over every space's chain, registered beside `atoms` on each of the
store's connections:

```sql
-- `calls`, part 0, of one space: its rows as the store numbers them
SELECT c0, c1 FROM kept_facts WHERE space = ?1 AND rel = 'calls' AND part = 0;
-- the rows whose first column is 42: a binary search where it leads the entry's order
SELECT c1 FROM kept_facts WHERE space = ?1 AND rel = 'calls' AND c0 = 42;
```

Columns `space`, `rel`, `ident`, `part`, `level` (or NULL), then `c0` to `c15` (NULL past the arity). `space`,
`rel` and `part` choose entries; `c0` narrows each. Only each key's newest entry is read; a gone one is absent.

## Required behavior, asserted (a test each)

- K-R1 `groups_reads_every_value` (`col`): `GROUPS` reads back every value of sorted, clustered and random
  columns, and is chosen for ids clustered by document.
- K-R2 `a_sorted_input_is_not_sorted_again` (`packed`): rows already in canonical order are packed without a
  sort, and equal the rows packed after one.
- K-R3 `levels_follow_their_rows` (`packed`): each row's level is read at its packed position.
- K-R4 `a_chain_reads_newest_first` (`kept`): a later file's entry replaces an earlier one's, a gone entry hides
  it, a reads-only entry reads the earlier rows with its own reads.
- K-R5 `a_space_appends_and_rolls_up` (store): a write appends one file of its puts only; small writes merge into
  the newest small file (64 writes, at most 2 files); a write as big as the chain merges every file, keeping every
  live entry, and a reader of the old chain reads it still; the swept files are the replaced ones.
- K-R6 `a_rolled_back_write_is_not_the_spaces` (store).
- K-R7 ep2: every kept entry's rows equal its frame's (as sets), and the chain's bytes are measured against the
  frames'.
- K-R8a `a_kept_relation_is_bound_from_its_chain` (store): a relation whose entry is its frame's is read from the
  chain (rows in canonical order, the frame's digest and reads, the entry's levels in its rows' order); a stale or
  missing entry's from its frame.
- K-R9 `a_frame_in_the_chain_is_its_metadata` (store): a frame put in the chain is written with `seg = -1` and no
  bytes; read from SQL and from the snapshot, its rows are its entry's; with no entry it is absent; an entry put
  as given (a granule, a row repeated) is read in its order, every row.
- K-R8a also: a levels part whose entry has no levels is absent (eight relations, so the map's order reaches a
  levels part after its part 0: ep2's k1 panicked there). K-R4's retag is put with no digest and keeps the entry's.
- K-R10 `kept_facts_are_a_virtual_table` (store): rows by space, `rel` and `part`; the newest entry; a gone one
  absent; `c0 = ?` a range (the entry leads by column 0); an entry put as given leads by none; columns past the
  arity NULL.
- `a_relation_started_from_a_view_has_its_version` (engine): a relation compacted from a view has its start, and
  its delta from the view, by version, relation and mark.
- `a_kept_input_is_decoded_where_it_is_first_read` (engine) also: `failed` is unknown before the input is read
  whole, and a failed input reads 0 within its count.
- K-R11 `a_relation_declared_sorted_is_laid_out_in_order` (engine): a relation declared sorted is in canonical
  order when done, dead rows left out, known sorted on its leading column.
- K-R8 ep2's rows are h1's (0 differ) with kept relations bound from the chain (step 5); K-R7 also finds no frame
  of a space with a chain that has no kept entry.

An open question for step 4: `GROUPS` costs a sorted id column about 9–11 bits a value (a group's 128 ids span
about 256); a line per group (its first value and slope, as `LINE` per column) would take it to a few bits, where
a delta varint takes 8. Whether it pays is for ep2's columns to say.

## Measured (ep2, 2026-11-18)

Four runs (k0 cold, k1–k3 one edit each), profiling build:

| | |
|---|---|
| rows against h1 | 0 differ, every run (K-R8) |
| K-R7 | 0 entries differ; 0 frames with no entry |
| relations bound from the chain | 28,938 of 29,732 of the cross space; 1,398 of 1,462 elsewhere (the rest: statistics parts, read as frames) |
| chain | 178 MB in 15 files over 9 spaces; the frames' segments 151 MB |
| bits a value | GROUPS 25.6 (40.4 M values, 129 MB); PACK 35.1; RUNS 5.4; LINE 27.5; CONST 0 |
| cross space | 3 files after 1,604 writes (tiered merging) |

The chain is 1.18× the frames: GROUPS holds ids at 25.6 bits where the frames' delta varints take about 8–16.
The open question below (a line per group, or splitting an id into document and local parts) is where the rest is.

Steps 6–7 (a fresh state, the same four runs):

| | step 5 | steps 6–7 |
|---|---|---|
| rows against h1 | 0 differ | 0 differ, every run |
| k0 | 1,393 G instructions, 2,113 MB peak | 1,256 G, 1,693 MB |
| k1, k2, k3 | 22.8, 22.2, 22.1 G; 583, 581, 569 MB | 21.2, 20.5, 20.7 G; 233, 226, 235 MB |
| frames | 151 MB of segments | 32,366 frames, every one metadata: 0 bytes, no segment |
| chain | 178 MB | 183 MB |
| bound from the chain | 28,938 of 29,732 | 29,304 of 29,732 (the rest levels and delta parts); none unbound with an entry |

The edit runs' peak fell by 60%: their kept relations are views of the mapped chain, not rows copied in.

## Steps

1. (built) `IntCol` `GROUPS`; `packed::pack_rows` (sorted check, levels); K-R1–K-R3.
2. (built) `kept.rs`: entries hold a `Packed` and levels; `KeptW` packs; the chain reader; K-R4.
3. (built) The store: `facts_chain`, a write appends, tiered merging, sweep; K-R5, K-R6.
4. (measured) ep2: the chain's bytes against the frames', K-R7.
5. (built) Bind kept relations from the chain in place of the frames; K-R8a, K-R8.
6. (built) The frames hold no rows (6a, K-R9); kept relations read in place (6b); the virtual table (6c, K-R10).
7. (built) Sorted outputs (K7, K-R11; Q217).
