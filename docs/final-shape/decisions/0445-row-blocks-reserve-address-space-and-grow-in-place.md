# 0445. Row blocks reserve address space and grow in place

Date: 2026-11-19. Status: decided (user, 2026-11-18: t49 §6 item 2, "then implement t49's second item"); built.

## Intro

A run's rows live in blocks of relations (`facts::rows::Facts`), shared by the relations of one life (a run's, a
batch's sources, kept rows). Until now a block was sized up front: 64 MB for a run's relations, 16 MB for sources and
kept rows (`FACTS_BYTES`). A tail that filled its block moved to another, one copy of its rows (0353 entry 5). t49
§6 item 2 asked for row blocks reserved large, wrapped once for the device, grown in place, their tails released, and
for the `FACTS_BYTES` choice to go.

Since 0435 §1 a block the device may read is host memory: anonymous, resident only where written, read by the device
through a no-copy wrap of its used extent. Two of item 2's parts were in place already:
- a block's pages are taken as rows are written, so an untouched reservation costs nothing (0435 R3);
- a dead relation's chunks go back to the block's free list, their pages to the OS (`Block::free_chunk`).

## Decision

1. **A block reserves address space enough to grow in place.** Where a reservation is address space (64-bit Unix),
   every block of relations reserves 16 GiB (`BLOCK_RESERVE`), whatever its life. No tail outgrows its block within
   a run's memory, so none moves. A Mac granted 4,000 such reservations at once; on Linux the mapping is made
   `MAP_NORESERVE`, so overcommit accounting does not refuse it. A relation alone in its block (a device run's own,
   one with index room) keeps a block of its size.
2. **Where a reservation is memory (WASI), blocks stay sized.** There `mem::reserve` is a zeroed allocation of the
   whole length, so the old sizes stay, and a full block's tail moves as before. `facts::rows::MOVED` counts every
   move and its bytes, and the profile prints them.
3. **The device's wrap still grows with the used extent.** It does not wrap the reservation once. Mapping a no-copy
   buffer into the GPU's address space costs about 20 ms a GiB at its first use, and more over pages never touched
   (0266 §12, measured). A wrap of 16 GiB would cost a block hundreds of ms. The wrap is remade at twice its length as
   the block grows (0435 §1). That is no copy: the rows stay where they are, and only the device's mapping grows.
4. **What a block holds is its used extent less its free list.** A block lives while any relation in it does, and no
   longer rotates when full, so its used extent would count its dead relations for its life. The run's count (0435
   §3, `budget::Written::host_blocks`) and a chunk's held bytes (0435 §6, `rows::Written::held`) take its free
   list's bytes out. Those pages were given back. A chunk's bytes written stay its blocks' used extents.
5. **The tails' release** is the free list's (`free_chunk` gives a dead chunk's pages to the OS). Pages past a block's
   used extent were never touched, so they are not resident.

## Requirements, each locked by a test

- **R1** A tail grows in its block past the old 64 MB without moving
  (`a_tail_grows_in_its_block_past_the_old_size_without_moving`).
- **R2** The count leaves out a chunk given back (`the_count_is_what_a_block_wrote_not_what_it_reserved`).
- **R3** A chunk's held bytes are the rows it handed on, not the extent of the block that holds them
  (`a_chunk_measures_what_its_blocks_wrote_the_same_every_time`).

## Measured (2026-11-19)

posthog/hogql cold, profiling builds, format 15, an M-series Mac at load 44-68, warm binaries; 1,629 findings each,
the same inline choices as 0444's runs in every one, no relation moved:

| | Instructions | Device refusals at run time (stratum runs) | Peak footprint |
|---|---|---|---|
| CPU, 0444 | 81.9-83.1 B | | 343-359 MB |
| CPU, this, 6 runs | 81.5-82.1 B | | 333-357 MB |
| Metal, 0444, 3 runs | 76.6-81.1 B | 14-49 | 1,408-1,643 MB |
| Metal, this, 8 runs | 70.9-72.6 B (one 75.5 B) | 2-6 | 1,327-1,629 MB |

- On Metal, the refusals were most of 0444's spread. They are the device's room refused at run time
  (`BackendFailure: an index of N rows`): the stratum then ran on the host.
- Still open (Q261): on Metal a chunk's held bytes differ between runs (14-36 MB for one chunk; the CPU's are the
  same in every run), so the chunks at once differ. The cuts and plans do not.
