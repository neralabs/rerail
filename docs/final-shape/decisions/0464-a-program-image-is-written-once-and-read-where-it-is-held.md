# 0464: A program image is written once, and read where it is held

Status: accepted (2026-11-19). Q26 (handed off by 0295 §6.1). Follows 0173, 0244, 0353, 0394; decides 0295 §6.3–4.

## Intro

A program image (0173) is the flat form of a rule program: about forty regions (strings, rule nodes, declarations,
facts, indexes) behind one header, each region at a 16-byte offset. Every version of a program (`Version`: a base, an
overlay of a delta on a parent, or a view that leaves rules out) reads its rules from one or more images.

Before this decision, an image was copied at least twice on its way from its writer to its reader:

- the writer staged each region in a vector of its own, then assembled the regions into a byte vector
  (`finish_bytes`), and `Version::open` copied those bytes again into an aligned `Vec<u128>`;
- a record that kept an image in the store (a prepared program, a planning choice, a pipeline step, a stored run, a
  module or library image) copied it once more into the record's byte area;
- a read from the store copied the source's bytes out of SQLite or out of its segment (`get_source`), and
  `Version::open` copied them into an aligned buffer, again.

Opening an image also walked every rule's node tree, which reads every page of an image a run may read a few rules
of (left by 0394).

Under 0353 every one of these is a copy, and none was in its list. This decision removes them.

## Decision

1. **The writer writes the image once, where it lives.** `Layout` takes each region's item count, and `place` makes
   the image's one buffer, 16-byte aligned, with its header written. The encoder then writes each region's slice in
   place (`Placing`), and `Placing::finish` makes that buffer the `Image`. Nothing is staged or assembled. The
   format is unchanged (`FORMAT` stays): each region holds what the old writer staged, in the same order, so a
   store's or bundle's images written before still open.
   `Builder::image` borrows the builder; `Version::of_image` and `Version::over_image` open the image as written,
   with no bytes in between.

2. **A record carries an image as its maker holds it.** A record's item (0306) has pieces (`record::Bytes`, an
   `Arc<dyn AsRef<[u8]>>`), not one byte area. An image is a piece: the `Arc<Image>` its version holds. The writer
   reads the image there when it applies the record. `program_text::pack`, `record_runs`, `StoredSteps::keep` and
   `rr::keep_image` hand on pieces. `Record::record` and `queue_record` keep their one-buffer form for small
   entries, which are moved in, not copied.

3. **An image's open checks what keeps reads sound, and walks no trees.** Handles index the image safely; only
   strings are read unchecked (`text`). So every open checks the header, every region's width and bounds, every
   reference into another region, and the strings' UTF-8 (`Depth::Held`). Bytes the engine's own store or bundle
   holds were written by its writer from a builder's checked parts, so their rules' node trees and facts' cells are
   not walked again. Foreign bytes (`Image::open`: a test's, a tool's) are checked in full (`Depth::Full`). A debug
   build checks every image in full, both where it is written (`Placing::finish`) and where it is opened.

4. **A stored image is placed in a segment and read there.** The writer places images uncompressed in a segment
   (`put_sources_placed`, as 0394 places a stored run), so a read maps them (`ObjectBytes::Mapped`) and opens the
   version over the mapping (`Version::open_shared`). An image an older store holds inline is loaded from its row
   into a buffer at 16 bytes (`ObjectBytes::Loaded`, decompressed into it): a load from disk to the exact in-memory
   bytes, which 0353 does not count as a copy. The shipped bundle (0264) is mapped, each image at 16 bytes. Where no
   mapping exists (not unix), the bundle is read once into a 16-byte-aligned buffer. Every program image read goes
   through `images::read_image`/`read_images` or `program_text::stored`. None goes through `get_source`.

5. **There is one fold rule.** An edit becomes an overlay on its parent unless it is at least half the parent's size
   (`2 · size ≥ parent.size()`, `finish_as`). Then it folds into a new base. This is the same geometric rule as 0456
   K4's merge of kept files (merge the newest files that weigh no more than twice what joins them): each layer is at
   most half of what is beneath it, so a chain of `n` edits has `O(log n)` layers, and folding costs `O(1)`
   amortized per edited rule. Nothing else folds.

6. **Kills are by range, and a view stays its own form.** An edit that only removes rules is a view: the parent's
   rule ranges it keeps (`view`, at most `max_ranges` of them), with no bytes. 0244's split parts are views of this
   kind. Expressing a view as a delta with an empty body (one base plus one delta) would cost the delta's header,
   about 1 KB (16 + 24 × 40 bytes), and an image to write, store and map. dl1 made 2,089 views in one cold run
   (0173). So views stay a second form: a delta only for an edit that adds rules or states parts.

7. **A found step names its sources (Q146).** A run that finds a kept pipeline step names the step's facts and its
   image in its manifest (an item of its record, named as the writer applies it), as the run that kept it did. So a
   collection between the two runs keeps them. The keep names what it stores in the same way.

## What remains

- `Image::open` copies foreign bytes into an aligned buffer. No engine read reaches it. Tests and tools that hold
  bytes in a `Vec` use it.
- `rr::images::read` copies a group of automata or a phase 0 graph out of the store. Those are entries their
  readers decode, not program images. Moving them onto blocks belongs to their own owners (the vocabulary's DFA
  cache, phase 0's graphs).

## Checks

- `program_text::tests::a_stored_program_is_read_from_its_image`: a stored image is placed in a segment and read
  there.
- `program_image::tests::a_stored_run_is_read_in_place`: a stored run's images, rows and facts are read in place,
  and an older store's inline image is moved into the segment by the keep.
- `rr::tests::library_modules_link_to_the_lowering_of_them_together`: module images are written in place and read
  back as held bytes, and they link to the same library.
- `program_image::tests::an_open_rule_writes_what_the_rule_builder_writes` compares two writers' bytes, and the
  manager's tests compare each version with a run from scratch by digest and size.

## Measured

The dl1 44-policy batch, as 0276 §3 ran it (profiling build, images required, CPU, strict, a fresh store, the M3
farm host at load 35–53), at a282371e9:

| | a26e71b5d (0276 §3) | a282371e9 (this decision) |
|---|---|---|
| findings | 48 | 48, in each of the three runs |
| cold | 81.7 G instructions | 79.9 G (20.8 s wall at load 53) |
| warm, first | 5.7 G | 3.7 G |
| warm, second | 3.3 G | 3.3 G |

So the batch costs no more than before. The two columns are many commits apart, so the difference is not this
decision's alone. Q401 records the one test that got slower on Linux, which waits on the store's msync, not on an
image.
