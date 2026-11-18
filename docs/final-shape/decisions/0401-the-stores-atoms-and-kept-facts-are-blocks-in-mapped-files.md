# 0401: The store's atoms and its kept facts are blocks, in files that are mapped

Status: decided by the user (2026-11-18: "everything should be in blocks all the time"; "these should be blocks …
I would prefer memory-mappable files than blobs"). Builds 0266's `Atoms` and `Facts` as what the store keeps.
Replaces the atoms' segment rows and key (0049, 0111) and the cross part's frames (0104, 0295 §4) as storage.
Steps 1–3 (atoms) built; 4 next; 5–10 (facts) after.

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

### Interface (rerail-blocks)

```rust
/// A kept relation's key.
pub struct FactKey<'a> { pub rel: &'a str, pub ident: &'a [u8; 32], pub part: i64 }

/// A kept relation, in place: its key, its metadata, and its rows as a view.
pub struct Kept<'b> { pub key: FactKey<'b>, pub digest: u64, pub reads: &'b [Read], pub types: &'b [u8], pub rows: relation::View<'b> }

/// What reads a space's kept facts (a base, or a base and its delta read as one).
pub trait KeptRead {
    fn get(&self, key: &FactKey) -> Option<Kept<'_>>;
    /// Every entry for `rel`, in key order (its parts).
    fn of(&self, rel: &str) -> Box<dyn Iterator<Item = Kept<'_>> + '_>;
    fn keys(&self) -> Box<dyn Iterator<Item = FactKey<'_>> + '_>;
}

/// The next delta of a space: written behind the run by the store's writer.
impl KeptW {
    pub fn create(path: &Path, base: Option<&KeptFacts>) -> io::Result<KeptW>;
    /// A relation the run derived (frozen in its own `Facts` block), copied whole.
    pub fn put(&mut self, key: FactKey, digest: u64, reads: &[Read], types: &[u8], rows: &Relation);
    /// An entry of the previous delta, kept as it is.
    pub fn carry(&mut self, kept: &Kept);
    pub fn finish(self) -> io::Result<KeptFacts>;
}
```

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
4. Measured on ep2: a new process's atoms (0.046 s, 215 MB today), its cold and warm runs.
5. `rerail_blocks` kept facts: the directory, `KeptW`, `KeptRead` over a base and its delta; F-R1 at the block.
6. The keep (`retain::write_frames`, `write_input_frames`) writes a space's delta.
7. The read (`preload_frames_live`, `kept_inputs`, the clean relations) binds views; F-R1–F-R3.
8. Collection as a roll-up; F-R4.
9. The virtual table; F-R5. The cross part's `frames` rows retired.
10. Measured on ep2 (the kept frames 0.064 s, the cross space's size).

## Measured

To fill.
