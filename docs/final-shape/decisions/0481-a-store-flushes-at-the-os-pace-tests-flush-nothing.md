# 0481: A store flushes at the OS's pace; tests flush nothing

Status: built (audited 2026-11-23). Accepted (2026-11-19). Q361. Follows 0266 (blocks in mmap files), 0402/0456 (kept
facts), 0295 (the deferrable tier; cited as 0440 on Typing-support-final-shape, whose tree has no 0440). Numbered 0474
there; 0481 here (roll-up 14).

## Intro

A store writes two kinds of things to disk: SQLite's rows, and the blocks those rows name (kept facts, atom tables,
segments of images and frames, deferrable slabs), each a file mapped in memory. Before this decision every block,
once written, was flushed and waited for: `msync(MS_SYNC)` over its whole mapping, then `fdatasync` (a barrier on
macOS). SQLite ran at its default `synchronous`.

That wait is what made `rr::tests::library_modules_link_to_the_lowering_of_them_together` take 217 s on a DGX Spark
(Q361): each phase 0 graph run kept its facts, and the writer msynced the kept file's reservation, 460–630 MB, of which
a few pages were written. The test's thread waited on the writer.

A test's store is thrown away when the test ends, and a run's store holds what can be derived again. Neither needs
the disk to have the bytes before the run goes on.

## Decision

1. **A store has a flush mode** (`rerail_blocks::block::Flush`):
   - `Synced`: every write waits for the disk. Blocks are msynced and `fdatasync`ed (`F_BARRIERFSYNC` on macOS);
     SQLite runs with `synchronous = FULL`.
   - `Os` (the default): nothing waits. The pages are the OS's to write back at its own pace; SQLite runs with
     `synchronous = NORMAL` (in WAL it syncs only at checkpoints).
   - `None`: nothing is flushed at all; SQLite runs with `synchronous = OFF`.

   `RERAIL_FLUSH=synced|os|none` chooses it when a store opens (`connection::store_flush`). Any other value is an
   error.

2. **The mode lives on the store's connections.** It is a property of a store, not of the process: the store opens
   every connection with the mode's `synchronous` (`configure_connection`), and code that writes a block reads the
   mode off the connection it writes with (`connection::flush_of`, from `PRAGMA synchronous`). `AtomsW`,
   `KeptW::finish`, `Sealed::write_back`, the segment writers and `sync_dir` take the mode as an argument. Nothing
   holds it globally.

3. **Tests flush nothing.** The engine's test binary sets `RERAIL_FLUSH=none` before `main`, unless the run names a
   mode (`src/test_flush.rs`, beside the watchdog). The store's own crate defaults to `None` under `cfg(test)`. A test
   that checks durability passes `Flush::Synced` itself, as the blocks crate's tests do.

4. **A synced block syncs the pages it wrote**: `msync` over `used`, rounded to a page, not the reservation.

5. **One exception: a GPU-wrapped slab on macOS** (0295) still writes itself back with its msync/`mincore` loop in
   every mode. The OS never writes wired pages back, so without it the bytes would not reach the file at all.

## What `Os` and `None` give up

Both survive the process's exit: the pages are in the OS's page cache, and the OS writes them. Neither survives a
power loss or an OS crash. After one, SQLite at `NORMAL` may have committed rows that name a block whose pages never
reached the disk, and a block's header may say frozen over pages that are zeros. A store that must survive that runs
with `RERAIL_FLUSH=synced`.

A store's contents can all be derived again, so the default takes speed. Left open: a guard that makes a store opened
after a reboot check its blocks, or drop what the last boot wrote (a boot id beside each commit). It would make `Os`
safe against a power loss.

## Checks

- `deferrable::tests::a_slab_written_back_by_the_os_opens_frozen_with_its_bytes`: a slab written back under `Os` waits
  for nothing, yet opens frozen with its bytes.
- `test_flush::tests::a_tests_store_flushes_nothing`: a test's store opens with `Flush::None`, and its connections
  say so.
- The blocks, store and logic crates' tests pass with their writers' new argument: 737 on the farm's Macs, 721 on a
  DGX Spark (Linux).

## Measured

`library_modules_link_to_the_lowering_of_them_together`, debug, on the M3 farm host:

| | wall | CPU (user + sys) | instructions |
|---|---|---|---|
| before, `synced` (load 7–34) | 140.6 s | 74.1 + 27.7 s | 1.00 T |
| `none` alone (load 7–34) | 140.8 s | 86.2 + 27.3 s | 1.00 T |
| `none`, every pack's graphs in one phase 0 run (load 8–17) | 22.3 s | 19.9 + 1.1 s | 262 G |

On a DGX Spark (Linux, debug, load 1.1), where the msync was the wait:

| | wall | CPU (user + sys) |
|---|---|---|
| before (Q361, at f9c3bd713) | 217 s | 87 s user, 40 s sys |
| this decision, `none` | 19.8 s | 22.7 + 0.7 s |
| this decision, `synced` (used pages only) | 19.3 s | 21.8 + 0.6 s |

On the M3, the flush alone saved nothing: macOS's msync of clean pages is cheap, and that test's time was in its phase
0 runs, one per program. The test now derives every pack's include graphs in one phase 0 run (`rr::modules_of`,
as a corpus run does with `phase0::graphs`), and reads the programs on the pool. On Linux, where the msync was the
wait, both help. At about 20 s (24.6 s beside other tests on the M1 Max) the test stays in the slow tier, without the
longer time limit Q361 gave it.
