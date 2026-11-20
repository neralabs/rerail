# 0394: A stored run is read where the store placed it

Status: built (2026-11-18), measured on ep2 below. Follows 0385 ("What it does not do").

## Intro

An edit's combine opens the tail's last run from the store (0393, 0385). On ep2 that run is 11 versions, 8 of them
images, with its analyses' rows (2.9 MB) and its facts (0.48 MB). Reading its 8 images took 0.019 s.

Each image was kept the way a file's source is kept: inline in its row, zstd-compressed. So every read made two
queries (the alias, then the frame), decompressed the frame, hashed the bytes with sha256 against the hash it is
stored under, copied them into the store's decompressed cache and out again, and then `Image::open` copied them once
more into an aligned buffer. The rows and the facts were read the same way. None of that work is needed to read
bytes the store wrote itself.

The store already keeps a run's parsed files that way (0295 step 5): uncompressed, in a segment beside its file, read
there mapped. This puts a stored run in a segment too.

## Decision

1. **A stored run's images, rows and facts are placed in one segment, uncompressed** (`program_text::store_run`,
   `ContentStore::put_sources_placed`). They are written in one segment write and one transaction, and the segment
   is named once that transaction commits. If the store already holds an image inline (a prepared program's, kept
   by `store_encoded`, or one the run was opened from), it is moved into the segment: its row now points at the
   segment, its id stays the same, and its bytes are read once to do it. A source the store already holds placed
   keeps its place, and no new segment is written for it. An alias that names other bytes is refused.
2. **A stored run is read in one query, in place** (`ContentStore::sources_by_alias_in_place`, `StoredRuns::open`).
   Every image and the analyses' rows are asked for together. A placed source comes back mapped
   (`ObjectBytes::Mapped`): it is not decompressed, not hashed and not copied. The check that covers it is the
   segment's own check when it is mapped. A source still held inline is decompressed and checked against its hash,
   as `get_source` does.
3. **An image opens over the mapping** (`Image::open_shared`, `Version::open_shared`, `Version::over_shared`). An
   image's bytes are either a buffer of its own or shared bytes held at a stable address (`SharedBytes`). Shared
   bytes must sit at 16 bytes. A segment places each entry at 64, and the mapping is page-aligned. The image's
   checks of its header, regions, references and rule trees still run (`Image::checked`): a handle reads the image
   without checks of its own.
4. **How a run was read is stated** (`StoredRuns::open_counted`, `Opened`): its images, how many were read in place,
   and whether its facts and rows were. The profile's "a stored run, opened" says the same.

## Required behavior, asserted

- `content_store::object_segments::tests::sources_put_placed_are_read_in_place_by_alias`:
  - a source held inline reads decompressed;
  - put placed, it is moved, keeps its id, and is read mapped at 16 bytes, its row a pointer with no frame;
  - one segment holds both sources, and putting them again writes none;
  - an alias of other bytes is refused;
  - a new process reads the sources in place.
- `program_image::tests::a_stored_run_is_read_in_place`:
  - the input's image, held inline first, is moved by the keep;
  - reopened, every image, the rows and the facts are read in place (`Opened`), and the input and output are open
    over the mapping (`Version::is_shared`);
  - a rerun over an edit makes a run from scratch's program and request record;
  - a second keep under the same key is no error, because a key's facts are kept once.

## Measured

ep2 on the m3 (300 files, 317 rules, `RERAIL_PREPARE=parts`, an edit of B101's body, profiled), against 0385 (e106):

| part | 0385 (e102/e106) | now (e121) |
|---|---|---|
| the last run, from the store | 0.032 s | 0.022 s |
| its images and rows read | 0.019 s (8 images) | under 1 ms (one query, mapped) |
| its images opened | 0.007 s | 0.019 s |
| its facts read and decoded | 0.002 s | 0.001 s |
| `combine` | 0.23 s | 0.24 s |

- All 8 images, the rows (2.9 MB) and the facts (0.5 MB) were read in place on every edit (e117–e121). Rows were
  1,334 with none differing from h1, `RERAIL_COMBINE_TAIL=check` passed (e120), and every pass of the tail ran over
  the edit.
- Most of the read's cost moved into the open rather than going away. A mapped image's pages are touched first by
  the image's check, so their page faults now count under "opened". The open as a whole is 0.010 s less.
- The program grew in the hub's merge (the bound program has 5,930 rules, up from 5,806; the merge works on 9,535,
  up from 9,300). That growth, not this change, accounts for `combine`'s 0.01 s.

## What it does not do

- An image's checks still read every rule's node tree, and so every page, when it opens: 0.019 s for 8, the
  pages' first faults included. A check kept with the image that wrote it, or checks made lazily per rule, would
  remove that.
- Other images (a prepared program's, a keyed step's) are still kept inline. They are moved only when a stored run
  names them.
