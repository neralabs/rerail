# 0456: Kept facts are packed chunks, appended to a chain of mapped files

Numbered 0402 where it was written (the sibling branch `final-rerail-shape-rr-align`), beside 0402 "Byte-pair
rules are one lookup"; renumbered 0442 when merged, and 0456 when that number turned out to be taken
(cost-classifier's 0442, "no process-global state"), past the highest number in any worktree. The sibling branch
renumbered it 0417 at its own hub merge (c8ec6ba92); merged back (535cc2c8b), it is 0456 in both, and a citation of
0417 or 0442 for the kept facts on either branch is this decision.

Status: built (audited 2026-11-23). Decided by the user (2026-11-18: "I would prefer a single format if possible … We
certainly shouldn't be storing 8-byte quantities … please go ahead and design and build this"). Replaces 0401 F1, F2
and F6's copy of a run's relation as it lies (8-byte cells in doubling chunks, its hash indexes, a cumulative delta).
Keeps 0401's intent (F3–F5): the store's kept facts are blocks in mapped files, SQLite holds only which files, and
readers read them through a virtual table. Steps 1–7 built (2026-11-18).

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

**A column's form** (2026-11-18, from ep2's columns, below): a column of a `Packed` is a `PCol`, one of four forms,
the smallest by exact size:

| form | holds | for |
|---|---|---|
| whole | one `IntCol` of the values | most columns |
| split | `v >> 18` and `v & (2^18 - 1)`, each an `IntCol` | node ids: `document << 32 \| slot << 18 \| index` (0083 §3) |
| whole, relative | `v - c0` (column 0's value at the row) | a column near column 0's, as a child is its parent's |
| split, relative | each part less column 0's same part | ids in the row's first id's granule |

A node id's high part is its granule (its document, a digest-like number, and its slot), its low part its index
there. A group of 128 ids that crosses documents spans 2^32 to 2^63, since documents are numbered as digests are;
its granules are a few runs and its indexes 18 bits at most. Column 0 is never relative. A probe of the order
searches its columns in turn, each within the range of the ones before (`Packed::probe`; `ColDec::equal_range`,
the high part's range then the low part's), so where column 0 leads the order (a kept entry's order is its columns
in turn) a later order column is searched where column 0 is one value: there it is the column less a constant, in
the column's order, and searched as such (`ColDec::held`). A whole relative column searched so is taken only where
no difference wraps. An order that column 0 does not lead keeps its columns absolute. The bit is the engine's `granules::INDEX_BITS`, which asserts it equals `packed::ID_SPLIT`. Every value
reads at the same cost: one or two packed reads, and column 0's where relative.

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

**Merge joins** (2026-11-18, Q251). A join of two relations in order on the join key is a merge: the outer side
scanned in order and the inner side read forward from where the last key's rows began. The engine had both
halves: a scan whose rows are known sorted on the next probe's key is not sorted again (`scan_order`, decision
0187), and a probe of a sorted order keeps a cursor (`candidates()`' hints, `Index::find_at`). Two gaps stopped a
kept relation read in place from taking part:

- it recorded no order, so the run knew nothing of it as sorted. `Source::order` gives a source's order (a kept
  entry's: its order key, then the rest), and `InRel::over_source` notes the source known sorted on it
  (`runs::Sorted::whole`; on its columns but the last, as a relation laid out in order is: an order on every
  column is the dedup order, which leads with a hash);
- a probe of its leading column searched the whole column every time. `Source::sorted_range_at` searches from the
  last probe's start (`ColDec::equal_range_from`: a gallop forward, then a search of that stretch), so keys in the
  column's order read it once.

A merge is then what the plan does when both sides are sorted on the key, and nothing chooses it apart: the
cursor costs a probe out of order no more than one whole search.

**Which relations declare it** (2026-11-18, from ep2's traces, `RERAIL_SPIKE_INDEX`, `RERAIL_SPIKE_PRESORTED`,
profiling build): none of ep2's yet. A whole run (k0, 191 s wall, 213 s CPU) builds 4,985 orders over 30.1 M rows
in 1.5 s; the most costly are on a second column (`rr_up107`, `rr_up108`, `unparen`, `type_bare` and others
probed by `[1]`, 0.9 s), which a canonical layout does not serve. Orders on a leading column take 0.2 s. Every
sort the run makes (orders, wave batches, scans for the next probe) is under 4.1 s of its CPU, 2%. The runs after
an edit (k1–k3) build no order and sort nothing. A declaration earns its sort where a relation is probed on its
leading columns by many later strata or runs; ep2 has no such relation.

**An order nobody declared** (2026-11-18): not kept beyond what the engine keeps now. A wave's batch is written in
its dedup order, which leads with a hash (`runs::tagged`), so a relation rarely ends in canonical order unless
declared; what the run already takes as known sorted (75.5 M rows of component orders and 20.7 M of wave batches
in k0) is taken without a sort. What is left is the 2% above, none of it after an edit. The trace that decides
it again is the presorted report: its sorts' rows wholly in order, and their time against the run's.

### K8. The kept facts in SQL

`kept_facts` is an eponymous virtual table over every space's chain, registered beside `atoms` on each of the
store's connections:

```sql
-- `calls`, part 0, of one space: its rows as the store numbers them
SELECT c0, c1 FROM kept_facts WHERE space = ?1 AND rel = 'calls' AND part = 0;
-- the rows whose first column is 42: a binary search where it leads the entry's order
SELECT c1 FROM kept_facts WHERE space = ?1 AND rel = 'calls' AND c0 = 42;
```

Columns `space`, `rel`, `ident`, `part`, `level` (or NULL), `id` (the row's place in its entry, from 0), then `c0` to
`c15` (NULL past the arity). `space`,
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
- K-R12 `a_source_in_order_is_known_sorted` (engine): a source that records its order is known sorted on its
  leading columns over every row; one that records none is not.
- K-R13 `node_ids_split_at_their_granule` (blocks): each column form reads every value; a probe from a cursor
  answers as a whole search for keys in order, out of order, repeated and absent; an order column after column 0
  is relative and searched within column 0's range, a key past every value included.
- K-R8 ep2's rows are h1's (0 differ) with kept relations bound from the chain (step 5); K-R7 also finds no frame
  of a space with a chain that has no kept entry.

An open question for step 4 (answered 2026-11-18, K2's column forms): `GROUPS` costs a sorted id column about
9–11 bits a value; a line per group, or an id split into its document and local parts, might take it near a delta
varint's. ep2's columns (`chain_columns_by_candidate`, over the 183 MB store) said: of 170.0 MB of columns, a line
per group saves 1.4%; a split at 32 (document, index) 10%; a split at 18 (granule, index) 30%; that split with each
part less column 0's 43% (97.6 MB), against a delta varint's floor of 90.8 MB, which has no random access. GROUPS'
widths showed why: 15.0 M of its 40.4 M values in groups 32 bits wide, 5.5 M in groups 52–63 bits wide.

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
The open question below (a line per group, or splitting an id into document and local parts) is where the rest is;
K2's column forms answered it (below).

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

K2's column forms and K7's merge joins (2026-11-18, a fresh state, the same four runs; the store's format 7):

| | steps 6–7 | ids split at their granule | and order columns relative, sources known sorted |
|---|---|---|---|
| rows against h1 | 0 differ | 0 differ, every run | 0 differ, every run |
| k0 | 1,256 G | 1,225 G | 1,228 G, 1,606 MB peak |
| k1, k2, k3 | 21.2, 20.5, 20.7 G | 21.1, 20.5, 20.6 G | 21.1, 20.7, 20.6 G; 227, 233, 230 MB |
| facts file | 183 MB | 140 MB | 120 MB |
| its columns | 170.0 MB | 117.8 MB | 97.1 MB (the best form for each, 94.6 MB; delta varints 89.9 MB) |

The edit runs' instructions do not move: they sort nothing and build no order (K7's traces), so a source known
sorted or a cursor's probe has no work to save there.

## Steps

1. (built) `IntCol` `GROUPS`; `packed::pack_rows` (sorted check, levels); K-R1–K-R3.
2. (built) `kept.rs`: entries hold a `Packed` and levels; `KeptW` packs; the chain reader; K-R4.
3. (built) The store: `facts_chain`, a write appends, tiered merging, sweep; K-R5, K-R6.
4. (measured) ep2: the chain's bytes against the frames', K-R7.
5. (built) Bind kept relations from the chain in place of the frames; K-R8a, K-R8.
6. (built) The frames hold no rows (6a, K-R9); kept relations read in place (6b); the virtual table (6c, K-R10).
7. (built) Sorted outputs (K7, K-R11; Q251).

## Merged with 0439 (2026-11-19)

Merged into the tree that had retired the frames (0439, 0401 step 9) ahead of this format. Kept from here: the
format whole (K1–K7: `Packed` entries, `GROUPS` and split columns, the chain `facts_chain` of files
`c<seq>-<tag>.blk`, tiered merging, the sweep, sorted outputs) and the store's floor 7 (`content-v7.sqlite`).
What 0439 supersedes, and how each step's intent is kept:

- **Steps 5 and 6a (binding through the frames, frames as metadata).** There are no frames. A run binds one commit
  of a space's chain (`retain::kept_read::Bound`, 0438): the chain's rows (`facts_chain`) are in the store's held
  snapshot, read before the first wave, so no SQL runs in the waves (0104). An entry is a relation's when its
  digest and width are the run's; nothing else is consulted.
- **Step 6b (read in place).** Kept: an entry with no levels is a view of its `Packed` where it lies
  (`kept_source::KeptSource`, `InRel::over_source`). An input's granules are one such view, each granule's rows in
  turn, in their order (was: read into one relation). An entry with levels is read into a relation with them, the
  one listed copy (`copy (0353 step 8)`). An entry whose atoms are another numbering's (past a fork, 0170) is
  derived again, as in 0438, not read through a map of ids.
- **Every frame write puts its entry (the `every_frame_write_puts_its_kept_entry` guard).** Nothing writes a
  frame, so the guard is gone; every keep is a `queue_kept`, and a wave's puts are one file a space
  (`KeptGather`, Q284).
- **Step 6c (the virtual table).** `kept_facts` as here (K-R10), with `kept_entries` beside it (0439 F-R5: each
  entry's key, digest, rows, types and reads). 0439's `kept_rows(space, rel, ident, part)` is `kept_facts`'s
  `space`, `rel`, `ident` and `part` constraints, and its `id` (a row's place in its entry) is `kept_facts`'s; the
  function went.
- **K-R8a and K-R9.** Their frame halves went with the frames; their store halves (levels follow their rows into
  canonical order; an entry put as given keeps its order, every row) are
  `levels_follow_their_rows_and_an_entry_put_as_given_keeps_its_order`. K-R7 compared the chain with the frames,
  and went with them (its measurement stands, above); the encodings' report (`chain_columns_by_candidate`) stays an
  `#[ignore]`d report over a store.
- **The lazy input (`Lazy`, a granule decoded where read).** Gone with the frames: a granule is a view where it lies.
- **Merged with 0439 (dc113d27e, 2026-11-19).** The 50 data-layer files, profiling build, load 18 on the M5: the
  chain takes 31.6 MB after a cold run and 38.9 MB after the edit's legs (5.0 MB of it files a merge replaced, which
  the next write removes), where 0401's blocks took 211 MB and the frames 39 MB (0439 "Met"); 589 outputs equal.
  Two faults the merge found: a fact relation's rows hashed in order, so a phase export maintained from kept rows
  (canonical order) renamed the cone's rounds' space (`facts_in_another_order_are_the_same_program`); and a round
  dropped a kept relation read in place as a view (`Source::kept`, `InRel::kept_in_place`).
