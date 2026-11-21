# 0401: The store's atoms and its kept facts are blocks, in files that are mapped

Status: decided by the user (2026-11-18: "everything should be in blocks all the time"; "these should be blocks …
I would prefer memory-mappable files than blobs"). Builds 0266's `Atoms` and `Facts` as what the store keeps.
Replaces the atoms' segment rows and key (0049, 0111) and the cross part's frames (0104, 0295 §4) as storage.
Steps 1–3 (atoms) built and 4 measured; 5 (the kept block) and 6 (the keep, beside the frames) built. **0456
replaces F1, F2 and F6's copy** (the kept relation as a copy of the run's, a base and one cumulative delta): kept
facts are packed chunks appended to a chain; steps 7–10 continue as 0456's 4–7.

## Intro

The store keeps two things that every new process needs before its first wave:

- **the atoms**, the strings every id names. Today they are rows of SQLite (`atom_segment`, 8 MB of strings a
  row) with a second table from each string's hash to its id (`atom_key`). A process copies the rows it needs into
  its heap and builds its own hash table over them. On ep2 that is every stored atom: 0.046 s and 215 MB at every
  start.
- **the cross part's kept facts**, the relations the last runs derived, so that an edit derives only what it
  changed. Today each relation is a frame: its rows encoded as varints, in a segment file (0295), found through a
  SQLite row per frame. A process scans 36–59k rows of its space and decodes each frame it binds into a structure of
  its own (0.064 s on ep2, 172–347 MB read).

Decision 0266 made both of these blocks: one mapping each, every structure nested inside, read where it lies.
They were built for what a run makes (`Facts`, `Chunk`, `Program`), not for what the store keeps. This decision
makes the store keep them as blocks too, in files beside the store, mapped by every process that opens it. SQLite
keeps only which files exist and how far each is committed.

## What it relies on

- **A block's layout** (0266 §2–§3): a header, a typed root, structures declared with `structure!`, references
  relative to their own address, no pointer outside the block.
- **Shared file mappings** (`MAP_SHARED`): a page written by one process is the page every other process maps.
  A reader reads only what a committed end covers, so it never reads a page being written.
- **One writer at a time** for the atoms: the atoms lease (0049), unchanged.
- **Durability by order**: a block's bytes are written and flushed (`F_BARRIERFSYNC` on macOS, `fdatasync`
  elsewhere) before the SQLite transaction that names them commits. The commit is the truth; bytes past a committed
  end are garbage, overwritten by the next writer.

## Decision: atoms

### A1. An atoms block

One structure, `AtomsRoot`, kind `ATOMS`:

| field | what |
|---|---|
| `first` | the first id the block holds |
| `tag` | its maker's random tag, which its row records too: a block not its row's is refused, not read |
| `cap`, `bytes_cap` | how many atoms and string bytes it has room for (its file is sparse) |
| `count`, `bytes_len` | how many it holds, written as it appends (only the committed end is trusted) |
| `ends` | `u64` a atom: where its string ends in `strings` |
| `hashes` | `u64` a atom: `hash_str` of its string (content keys, 0212; rehashing the table) |
| `strings` | the strings, back to back, UTF-8 |
| `table` | open addressing, `u64` a slot: the hash's high half and the id plus one (0 is empty); twice `cap` |
| `placed` | slots taken; an append refuses at three quarters (stale slots count, A3) |

A lookup hashes the string once, probes the table by the tag, and compares the string at the slot's id, in
place. A slot whose id is at or past the reader's end is passed over (another process's, uncommitted).

### A2. The store's atoms are a chain of such blocks

`<store>.atoms/` holds a **base** (`b<n>.blk`, frozen) and up to a few **deltas** (`d<n>.blk`), each covering
the ids after the one before. The newest delta is **open**: the lease holder appends to it. A delta that fills (its
`cap` or `bytes_cap`) is frozen and a new one, twice as large, is started.

SQLite keeps `atom_blocks(seq, name, first, upto, tag)`: each file, the end its last commit reached, and its tag. The
ids end at the largest `upto`. Atom 0, the empty string, is every store's and no block's. A new block is made under a
name of its own and renamed into place, so a file left at its name (a rolled-back transaction's, a deleted store's) is
never truncated under a process still mapping it.

### A3. Writing is appending, then committing

A flush (`write_pending`, in the transaction that writes rows naming the atoms, as 0049 requires) appends the
pending atoms to the open delta (strings, ends, hashes, table slots), flushes the file, and updates its row's
`end` in the same transaction. After the commit, the atoms below `end` are the store's: this process drops their
heap copies, since its mapping holds them. The commit hook of 0049 is unchanged: a transaction that writes rows
and did not flush is refused.

A crash or a rollback after the append leaves bytes and slots past the committed end. They are never read (A1), and
the next writer appends over them (`AtomsW::resume`). The slots stay: clearing one would end a probe early for a
reader probing past it now. A stale slot is passed over, since its string is compared; it only takes room (`placed`).

### A4. Reading is mapping

A process maps every block named in `atom_blocks`, read-only, up to the committed `end`. It holds no stored
string, hash or slot on its heap: only its pending atoms (heap, as now) and the mappings. `Atoms::str`, `get`,
`lookup`, `content_keys`, `with_str` and the `atoms` virtual table read the blocks in place. Another process's
commit is seen by reading `atom_blocks` again (`catch_up`, as `read_end` does now) and mapping a new file or
reading further into one already mapped.

So the lease's read whole (0316's 64 MB share), holding by segment (0111 §2), `hold_named`, `read_atoms` and
`held_below` (0397) have nothing left to do: every stored atom is held, by its mapping, from the open.

### A5. Roll-up

Under the lease, when the deltas are more than four or hold more atoms than the base, the writer writes a new base
holding every atom (table sized for all), replaces the rows in one transaction, and removes the old files after the
commit (a process mapping them keeps its mapping; unlinked files stay readable). Ids never change.

As built (`Stored::roll_up`): the writer is the store's write transaction, the same one that held the appends. It
reads every id through views of the rows as that transaction has them, and writes `b<seq>.blk` under a making name.
It freezes the file, renames it into place, then deletes the rows and inserts the base row, all in that transaction.
The old files are not removed in the roll-up itself: the transaction may still roll back. The next write that is not
a roll-up removes every file the rows do not name (`sweep`), inside its own write transaction. That covers a committed
roll-up's deltas, a rolled-back roll-up's base, and a crashed maker's file. The writing process reads through the base
at once. Another process re-reads the rows when it catches up, and maps what they name. A file named in rows it read
just before a sweep may already be gone, so a reader retries up to three times with the rows read again
(`Interner::advance_reading`).

### A6. Forks (0160, 0170) are unchanged

A process that gave out provisional ids while another wrote keeps its map between its ids and the store's. It finds
the store's ids of its strings by probing the mapped blocks (in place of `atom_key`). Its view past the fork follows the
rows exactly, its own appends included inside the transaction that made them, and shrinks with a rollback
(`Stored::view_rows`): a view widened there and left wide read a rolled-back append as stored, so it was never written
again. Below the fork every stored atom reads as itself, so the map's relinking below it (`below`, `relinked`) is gone.

### A7. An older store

The store's floor rises to 6 and its file is `content-v6.sqlite` (schema.rs: "a store is never upgraded"; the store is
a cache). An older store is neither converted nor read: at its own path it is refused (`an_older_floor_store_is_refused`),
and an older binary keeps its `content-v5.sqlite`. (This replaces the conversion first written here: the floor's rule
was already the store's, and a converter would be code for stores that no binary of this floor reads.)

A store is copied with its folder: the tests' template freezes its blocks and publishes them beside it, and each copy
clones them, as it now clones the tapes', frames' and memo's folders too (a copy without `.objects` had failed, a
source's segment not there); a capture copies them (`copy_blocks`). A reader maps its blocks read-only and keeps no
descriptor; the writer's is let go with the store's last handle.

### Interface (rerail-blocks)

```rust
/// What reads a run of atoms, wherever it lies (one block, the store's chain).
pub trait AtomRead {
    /// The ids it holds: `first..end`, `end` the committed end.
    fn ids(&self) -> std::ops::Range<u32>;
    fn str(&self, id: u32) -> Option<&str>;
    fn hash(&self, id: u32) -> Option<u64>;
    /// The id of `s` (hash `h`), if it holds one below `ids().end`.
    fn find(&self, s: &str, h: u64) -> Option<u32>;
}

/// An open atoms block in a file: one writer, the lease holder.
impl AtomsW {
    pub fn create(path: &Path, first: u32, tag: u64, cap: u32, bytes_cap: u64) -> io::Result<AtomsW>;
    /// The open block for its next writer, from the committed `end` (`resume`).
    pub fn reopen(path: &Path, end: u32) -> io::Result<AtomsW>;
    /// Continues from the committed `end`: appends past it taken back, their slots left (A3).
    pub fn resume(&mut self, end: u32) -> io::Result<()>;
    /// Appends `s`, `None` when full (the caller starts the next delta).
    pub fn append(&mut self, s: &str, h: u64) -> Option<u32>;
    /// Every append so far on the disk: before the commit that names them.
    pub fn sync(&self) -> io::Result<()>;
    pub fn freeze(self) -> io::Result<()>;
}

/// An atoms block mapped read-only, read up to a committed end.
impl AtomsV {
    pub fn open(path: &Path, end: u32) -> io::Result<AtomsV>;
    pub fn extend(&mut self, end: u32) -> io::Result<()>;
    /// Reads to `end`, nearer than before too (a fork's view after a rollback, A6).
    pub fn set_end(&mut self, end: u32) -> io::Result<()>;
}
```

The store's chain (`content_store::atoms::Stored`) implements `AtomRead` over its blocks; the `Interner` reads
stored ids through it and pending ids from its heap.

## Decision: kept facts

### F1. A kept `Facts` block

0266's `Facts` block with a directory at its root. Each entry is what a frame's row and bytes hold today:

| field | what |
|---|---|
| `rel`, `ident`, `part` | the relation's name (in the block's string table), its identity, its partition (`0` whole, documents, a granule) |
| `digest`, `reads` | its rows' digest, and each relation it was derived from with that read's digest |
| `types` | each column's value type |
| `rows` | a `relation::Relation`, frozen: its columns (packed, 0266 §14), canonical order, hash indexes, statistics |

The directory is sorted by (`rel`, `ident`, `part`), so an entry is found by a binary search in place, and a space's
entries for one relation are adjacent (the documents' frame and its granules).

### F2. A space keeps a base and one cumulative delta

`<store>.facts/<space>/b<n>.blk` and `d<n>.blk`. A run that changed nothing writes nothing. A run that changed
relations writes a new delta: the relations it derived or whose reads changed, copied whole from its run's `Facts`
block (a frozen relation is a range: `memcpy`, 0266 §3), the previous delta's entries it did not replace carried
the same way, and a directory over both. Its base stays. When the delta has cost what a roll-up costs (0266 §6),
the writer rolls the two into a new base. SQLite keeps `facts_blocks(space, base, delta)`, replaced in the
transaction that commits the run's writes.

### F3. Binding a kept relation reads it where it lies

A new process maps its space's two blocks. A kept relation it binds (a clean derived relation, a kept input, a
delta's base rows) is a `relation::View` over the mapping: no row is decoded or copied (0353). The digests, reads
and granules the run checks are read from the entry. The relations of a stratum the run derives again are written
into its own `Facts` block as now.

### F4. Collection is a roll-up

What a space keeps stays the kept manifests' and live programs' (0227). Collection writes the next base without the
entries no live program reads, instead of deleting frame rows one at a time (Q19's step 2, which deleted
overflow chains row by row). The space's size is then what its live programs read.

### F5. Queryable from SQL

Each space's kept relations are readable through a virtual table over the mapped blocks, as the atoms are.

### F6. What the keep puts, and how (step 6)

- **A derived relation** is the run's `InRel`: its rows as one relation (`Rows::in_block`: the one part, or its
  parts merged once), its dead bits, and its rows' levels (decision 0090). The entry holds all three. The rows are
  copied by `Relation::try_copy_into`, which needs only a relation nothing writes meanwhile, not a frozen one: a run's
  parts are written in any order and carry the indexes the run added, and the copy keeps both.
- **Atom ids.** A run's `s` columns hold the ids the store gives them, except in a forked process (0170), whose ids
  past the fork are its own. A forked process's copy is mapped in place, in the kept block, through the fork's map,
  column by column over the chunks: the rows are still copied, not encoded. A frame was written as the store numbers
  it for the same reason (0212).
- **An input's granules** are rows the run read from its sources, not a relation it froze. Each granule's rows are
  pushed into a relation of the kept block (`KeptW::put_rows`), one write, with no index.
- **Reads changed, rows the same** (a frame's retag) carries the previous entry with the new reads. **A relation
  changed by a delta** (a frame's delta after its frame) is put whole: a copy costs what its bytes do, where an
  encode cost what its values did.
- **The write** is the store writer's, behind the run, in the transaction that commits the run's other writes. It
  writes the space's next delta: the previous delta's entries not replaced are carried, and the run's are put. The
  temp name, rename and sweep are the atoms' (A2, A5). `facts_blocks(space, base, base_tag, delta, delta_tag)` is
  replaced in the same transaction. The job holds the run's relations (their blocks' `Arc`s) until the copy.
- **Alongside the frames until step 9** (retired, 0439). Each place a frame is written also puts the entry, so that step 7 can bind
  from the blocks and be checked against the frames (F-R2) before the frames are retired. Every copy is placed
  before any original is removed.

### Interface (rerail-blocks), as built in step 5 (`kept.rs`)

```rust
/// A kept relation's key; ordered as the directory is.
pub struct FactKey<'a> { pub rel: &'a str, pub ident: &'a [u8; 32], pub part: i64 }

/// A kept relation, where it lies: its key, what a run checks of it, and its rows (`view()` reads them).
pub struct Kept<'b> { pub key: FactKey<'b>, pub digest: u64, pub types: &'b [u8], pub rows: &'b Relation, .. }
impl<'b> Kept<'b> { pub fn reads(&self) -> impl Iterator<Item = (&'b str, u64)>; pub fn view(&self) -> View<'b>; }

/// What reads a space's kept facts: one block (`KeptFacts`), or a base and its delta read as one (`KeptView`).
pub trait KeptRead {
    fn get(&self, key: &FactKey) -> Option<Kept<'_>>;
    /// Every entry for `rel`, in key order (its parts).
    fn of<'s>(&'s self, rel: &str) -> Box<dyn Iterator<Item = Kept<'s>> + 's>;
    fn all(&self) -> Box<dyn Iterator<Item = Kept<'_>> + '_>;
}

/// A frozen kept block, mapped read-only; its kind, state and tag checked against the store's row.
impl KeptFacts { pub fn open(path: &Path, tag: u64) -> io::Result<KeptFacts>; pub fn base_tag(&self) -> Option<u64>; }
pub struct KeptView<'a> { pub base: Option<&'a KeptFacts>, pub delta: Option<&'a KeptFacts> }

/// A space's next base or delta, written under a making name and renamed into place by the store's writer.
impl KeptW {
    pub fn create(path: &Path, tag: u64, base_tag: Option<u64>, reserve: usize) -> io::Result<KeptW>;
    /// A relation a run derived (frozen in its own block), copied whole (`Relation::try_copy_into`).
    pub fn put(&mut self, key: FactKey, digest: u64, reads: &[(&str, u64)], types: &[u8], rows: &Relation) -> io::Result<()>;
    /// An entry of the previous delta, kept as it is.
    pub fn carry(&mut self, kept: &Kept) -> io::Result<()>;
    /// In a delta: the base's entry for `key` is no longer kept.
    pub fn gone(&mut self, key: FactKey);
    /// The directory sorted and written, the block frozen (truncated to what it holds) and on the disk.
    pub fn finish(self) -> io::Result<()>;
}
```

The copy is `Relation::try_copy_into(&self, block) -> Option<&Relation>`. It makes the descriptor again with the
source's widths, order and `min_log`, so each chunk covers the same rows. Each column's chunks, the kills, the dead
bits, every index's buckets and rows, and the statistics are then copied by `memcpy`, and each reference set to its
copy. Nothing is decoded, sorted again or rebuilt. A relation's pieces need not be one range of its block, so the
copy goes piece by piece.

## Required behavior, asserted (a test each)

Atoms:

- A-R1 `an_atoms_block_finds_what_it_holds`: appended strings are found by `find`, read by `str` and `hash`; a
  string not appended is not found; a full block refuses the next append.
- A-R2 `a_reader_reads_only_the_committed_end`: a view opened at `end` neither reads nor finds an atom appended
  past it, and does after `extend`.
- A-R3 `the_stores_atoms_are_mapped_not_read`: a new process's `Atoms` over a store of 100,000 atoms holds no
  stored atom on its heap (`bytes()` below 1 MB), and finds and reads every one.
- A-R4 `an_append_not_committed_is_not_the_stores`: atoms appended and the transaction rolled back are not seen by
  a new process, whose next atoms take their ids.
- A-R5 `another_process_commit_is_seen_by_catching_up`: atoms committed by another connection's process are read
  after `catch_up`.
- A-R6 `an_older_floor_store_is_refused` (A7): a store of floor 5 is refused at its path, not misread.
- A-R7 `a_roll_up_keeps_every_id`: deltas rolled into a base read the same; a view of the old files still reads.
- A-R8 the existing atoms tests (forks, reservations, the commit hook, the virtual table) pass.

Facts:

- F-R1 `a_kept_relation_is_read_where_it_lies`: a bound kept relation's columns point into the mapping.
- F-R2 ep2's rows are h1's (0 differ) through edits, cold and warm.
- F-R3 `a_run_that_changes_nothing_writes_no_facts`.
- F-R4 `collection_rewrites_without_dead_entries`.
- F-R5 the kept relations read through SQL equal their views.

## Steps

1. `rerail_blocks::atoms`: `AtomsRoot`, `AtomsW`, `AtomsV`, `AtomRead`; A-R1, A-R2.
2. The store's atoms on blocks: `atom_blocks`, appending in `write_pending`, mapping in `catch_up`, the `Interner`
   over `Stored`; A-R3–A-R5, A-R8.
3. (built) The roll-up and its sweep; A-R7. The conversion became the floor's rise (A7); `atom_segment`, `atom_key`
   and the holding paths went in step 2.
4. (measured, below) ep2: a new process's atoms (0.046 s, 215 MB before), its cold and warm runs.
5. (built) `rerail_blocks::kept`: the directory, `KeptW`, `KeptRead` over a base and its delta; F-R1 at the block
   (`a_kept_relation_is_read_where_it_lies`), `a_delta_reads_over_its_base`, `a_key_put_twice_is_refused`.
6. (built) The keep (`retain::write_frames`, `write_input_frames`; `write_kept`, `write_input_kept` since 0439) writes
   a space's delta, beside the frames.
7. (built, 0438) The read (`preload_frames_live`, `kept_inputs`, the clean relations) binds views; F-R1–F-R3.
8. (built) Collection as a roll-up; F-R4. Each program (its `prepared_key`) keeps a reader record in each space it
   reads: an entry of `READERS` listing the identities of the entries it asked and found, or put. A put's record
   goes in the same file as its entries; what a run only asked goes at its end, before its manifest. A record only
   grows: the writer adds a put of it to the one kept. Collection runs in the corpus collection pass, under its
   lease, after the keep decision and while the dead plans are still marked dead (`corpus_plans.live = 0`). It
   rewrites each space a dead program's record names as one base, without the entries only dead programs' records
   name and without those records, and drops a space left empty (`collect_kept_facts`). An entry no record names
   (an older build's, a session's) stays.
9. (built, 0439, 0456) The virtual tables (`kept_entries`, `kept_facts`); F-R5. The frames retired: the engine writes and
   reads none, the store's frame machinery is gone, and an old store's frame tables are dropped when it is opened.
   The blocks are still larger than the frames were (0439, "Open").
10. (measured, below) ep2: the kept reads 0.053 s (0.064 s the target), the cross space 160 MB, collection's roll-up.

## Measured

### Atoms (step 4, 2026-11-18)

ep2 (`tmp/m/ep2r`) on m3 with the profiling build, at load 22–23. The state is a fresh v6 store, because the v5 one
is refused (A7). The run `a0` is cold; `a1`–`a3` are warm, each with one rule's constant changed (`go.sh`). The
"before" column is `e152` from the hub merge's measurement: the same seed and binary flags, over the floor-5 store
(`ep2r-parts`). That store is older, and its history and frames hold more (364.7 MB of history and 59,374 frames,
against 44.4 MB and 31,914 here).

| | before (e152, warm) | after (a2, warm) |
|---|---|---|
| rows vs h1 | 1334, 0 differ | 1334, 0 differ (cold and every warm run) |
| instructions | 28.3 G | 21.6 G |
| CPU | 2.36 s | 1.65 s |
| wall | 2.05 s | 1.43 s |
| the atoms' lease span: allocated, kept | 215.2 MB, 58.3 MB | gone (no span; nothing held) |
| jemalloc heap at the end: live, resident | 251.8 MB, 312.8 MB | 169.0 MB, 238.7 MB |
| maximum RSS | 690 MB | 541 MB |
| page reclaims | 94,265 | 49,178 |
| peak footprint | 505 MB (529 MB by `time -l`) | 570 MB (598 MB by `time -l`) |

The cold run `a0`: 1,290 G instructions, 104 s CPU, 42.5 s wall, a 1,764 MB peak. It appended 477,430 atoms, which
went into four deltas: `d1`–`d4`, 65,536, 107,629, 168,807 and 135,457 atoms, in 90 MB on the disk. They were not yet
rolled up: with no base, a roll-up is due only past four deltas.

Open: the peak footprint rose 65 MB while the heap fell 83 MB and RSS fell 149 MB. The footprint is above the RSS, so
it counts pages that are not resident. The different store states may be the cause. So may the atoms writer's
writable shared mapping of `d4` (50 MB): the writer is opened only to append, and this warm run appended one atom.
Neither `footprint` nor `vmmap` can attach to a process over the farm's ssh session, so it is not yet known (Q230).

### The keep beside the frames (step 6, 2026-11-18)

ep2 on m3 with the profiling build, at load 40–60. A fresh store `k0` (cold), then `k1` and `k2` (warm). The old v6
store after its rules were restored: `a6` (cold again: new programs), then `a7` and `a8`.

| | rows vs h1 | instructions | CPU | wall | peak |
|---|---|---|---|---|---|
| k0 cold | 1334, 0 differ | 1,436 G | 139 s | 115 s | 2,051 MB |
| k1, k2 warm | 1334, 0 differ | 22.5 G, 21.7 G | 1.78 s, 1.71 s | 1.55 s, 1.47 s | 577 MB, 572 MB |
| a7, a8 warm | 1334, 0 differ | 25.2 G, 20.9 G | 2.02 s, 1.58 s | 2.07 s, 1.44 s | 415 MB, 413 MB |

The space's blocks on the disk are 641 MB, against 147 MB of frames. The cross space's base alone is 388 MB, and it
has two deltas of 136 MB, one of them left for the next write to sweep. A frame is zigzag varints of each column's
differences; a kept relation is 8-byte columns with their last chunk's room and their indexes. The cold run's
instructions rose 11% against `a0`, from writing both. Before the frames are retired (step 9), the copies must be no
larger than the frames: a kept relation's columns packed to their widths (0266 §14) when they are copied, and the
chunk room unwritten left out. A run handed off 0–5 relations before the keep, with their rows dropped; step 7 keeps
those at the hand-off.

### The reads bound where they lie (step 7, 2026-11-18)

Decision 0438 has the step's faults and fixes. The 57 data-layer rules over corpus0's first 50 files, with the
profiling build, at load 31 on the M5: cold 9.6 s, warm 0.1 s, an edit adding a function 2.0 s (2.2 s profiled
before), the edited tree cold 8.4 s. Its 589 outputs equal across the edit, the warm run after it and the cold run
(F-R2), and a run that changes nothing writes no facts (F-R3).

### The frames retired (step 9, 2026-11-19)

Decision 0439. The same 50 files with the profiling build, at load 36 on the M5: cold 7.5 s and 212.4 G
instructions (231.0 G with the frames), warm 0.1 s, the edit 1.7 s and 32.8 G (34.6 G) at a 547 MB peak (765 MB), the
edited tree cold 7.3 s. Its 589 outputs are equal (F-R2), with 48 findings. The kept blocks took 211 MB on disk, where
the frames took 39 MB; merged with 0456's chain (dc113d27e), the same runs keep 38.9 MB, 31.6 MB after a cold run
alone (0439, "Met").

### Collection and the kept data (steps 8 and 10, 2026-11-21)

ep2 on m3 with the profiling build (42990efb6d), at a fresh state, its rules renamed `.rerail` (PR #508's rename);
`tmp/m/ep2r/q-collect.sh`. Every run's rows are h1's (1334, 0 differ).

| run | instructions | CPU | wall | peak | kept facts | the cross space |
|---|---|---|---|---|---|---|
| c0 cold | 2,473.7 G | 178.6 s | 94.5 s | 1,727 MB | 88.9 MB, 12 files (the earlier sequence's cold run, below) | |
| e-qq990, the first under `RERAIL_PREPARE=parts` (another preparation: cold again) | 2,165.0 G | 162.2 s | 82.6 s | 1,736 MB | 175.5 MB, 15 files | 173.2 MB |
| e-qq991, one constant changed | 53.0 G | 4.39 s | 2.93 s | 429 MB | 162.3 MB, 13 files | 160.0 MB |
| e-qq992, one constant changed | 52.4 G | 4.24 s | 2.92 s | 426 MB | 162.4 MB, 13 files | 160.1 MB |
| w1 warm, deciding under a 1 MB history | 35.9 G | 2.66 s | 2.22 s | 378 MB | 162.3 MB, 11 files | 160.0 MB |

The roll-up at e-qq991's decision took the spaces from 175.5 MB in 15 files to 162.3 MB in 13: the entries only
dead programs' records named. w1's decision (manifests 10 to 7, live plans 34 to 27) rewrote two more spaces
(116 KB). The cold program's 86 MB of cross entries stay; c0's phase manifests (its root's protected lanes) are
still kept, and they name its plans. That was not checked entry by entry: the `sql` command takes no query of
`kept_entries`.

A warm run after collection derives nothing more: w2, profiled, has the same rows. Its kept reads: the cross
session's kept part 0.013 s, the kept inputs bound where they lie 0.030 s, the kept statistics 0.010 s (0.053 s).

Before 42990efb6d, a run collected nothing under the `logic` command. The command's analysis run (decision 0547)
holds a collection lease until it finishes, and a run's own pass (`record_run`) found it held every time. 13 runs
(the same cold run, then three edits and a warm run without `RERAIL_PREPARE=parts`) made no keep decision, and all
41 plans stayed live. `rerail-engine gc` on that store then decided (41 plans to 33 live). The roll-up took it
from 89.2 MB in 15 files to 88.9 MB in 12, the cross space from 86.9 MB to 86.6 MB. The warm run after took 33.0 G
against 32.7 G before. Now the analysis run collects as it finishes (`collect_after`).

Against 6fc580233a (the step 6 commit, measured again on the M3 the same day, profiling binary), the regressions
were three, each found in the two profiles:

| ep2, M3 | 6fc580233a | before the fixes | after |
|---|---|---|---|
| cold | 1,078-1,220 G | 2,476 G | 1,517 G |
| warm | 1.8 G | 35.9 G | 2.0 G |
| edit (`qq991`) | 43.9 G | 74-90 G | 75.8 G |

- Warm (7127549186): every `logic` command begins an analysis run (0547), and the whole-run cutoff skipped any run
  with a staged owner (since 395a6ac2f7). A hit now re-owns the recorded run's manifests.
- Cold, `open_tuple` (d92e01dffb): 0299's combination floor priced `child($t, $c, _)` at a row a pair after a cross
  product, and Q146's witness pricing then took the cross product as nearly free: 61 M rows, 19.7 s.
- Cold, prepare 27 s against 1.4 s (9baa291010): the push held back one reader of a helper shared by 400
  `violation` rules a round, so it ran about 400 rounds, each a split of the program.

Still above the baseline, by cause:
- The narrow copies of main's #923 (0c3dde2edb) double the program: 6,664 strata against 3,236, and
  `rr_narrow_<rel>` evaluates its original's rules again (`ruff_safe_operand` and its copy, 1.6 s each). The
  fixes that make a copy its original's rows (15e8c73191, c82460adf4) are on investigate-bend-vs-datalog alone.
- An edit counts Q396's semi-joins (`docof`, `end_line`, `visit_index`, `node_order`, `ast_depth`) by walking every
  held node's document, making all 300 (1.1 s of the edit's stats, 0.06 s before): their statistics are kept
  under the cross program's key, which an edit changes, not beside the base relations' table.
