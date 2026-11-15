# 0239: A program's output is a flat image, written by delta

Status: draft (2026-11-14); stages 1–4 built (`plan::output`). Builds on decisions 0173 (a program is a flat image),
0140 (the tape is the engine's bytes, read in place), 0209 (statistics are one flat table) and 0212 (kept frames
are of content). Follows 0230, whose "still open" list it replaces at its top.

## Intro

A run of a rule program writes each output relation as a text file, `<out>/<rel>.csv`: one line per row, columns
separated by tabs, a node as `path:local`, sorted. After 0230 and the hub merge, writing those files is the largest
fixed cost a one-file session has left. In a sample of O125 sessions 12–16 (491 samples), about 150 are spent on
it, against 9 on planning:

| where | samples | what |
|---|---|---|
| `decoded_outputs` | 42 | every output's rows read and formatted as text lines, then sorted |
| the writer's loop | 69 | every line split and parsed again, each node formatted as `path:local` |
| sorting the lines | 27 | as strings, a second time |
| parsing | 10 | `str::parse` of the line's own numbers |

Three things make it cost the whole output, not what changed:

1. **Text is the interchange form.** The memo's entries hold each file's output as lines (`memo::Entry::lines`).
   The cross part decodes its rows to lines (`decoded_outputs`). The writer parses those lines back to numbers to
   format their nodes. A row is printed, parsed and printed again.
2. **Every output is written whole.** A session skips a file only when its stratum was clean, and never for
   `finding` and `type_of`, which the run also reads. A one-file edit rewrites `type_of` whole.
3. **The format has no structure.** A reader has to parse the text to get rows back. Phases, phase 0, the typing
   bridge, constants folding and `test-rr` all split lines on tabs.

The engine's other data is no longer like this. A tape is a header and fixed-width columns, read in place (0140). A
program is a flat image of regions, keyed by ids, and an edit is a small image stacked on it (0173). The planner's
statistics are one flat table by relation number (0209). A kept frame is of content: its atoms are written as the
store numbers them (0212). The output should be the same kind of thing.

## Decision

### 1. Rows, not lines

An output row is its columns' values, as the engine holds them: an `i64` per column, typed by the column. No row is
printed until a reader asks for text.

| type | value | shown as |
|---|---|---|
| `text` | a string's index in the image (§2) | the string |
| `node` | `doc << 32 \| local`; below -1 a `@place`; -1 none | `path:local`, `guest<doc>:<local>`, `@place("path", line)` |
| `int` | the integer | the integer |

A symbol, a kind and a field are held as their text: content, whatever a process or a grammar numbers them (0212).
A value its column's type does not hold as it was written (`007` in an int column, `None` in a node column) makes
that column text, so every row shows as it was.

The memo's entries will hold a file's output rows flat, as they hold its exported rows, and the cross part hands
its rows as held (stage 2). The rows are gathered, deduplicated and sorted as numbers.

### 2. The output image

A run's outputs are one file of images, `<out>/outputs.rro`. Each image's layout follows the program image's:

```text
header   magic "RROUT\0\0\x01", format, region count, then per region: item width, item count, byte offset
         (16-byte aligned)
meta     a base or an overlay, and how the run shows its nodes (its own corpus's or T's; by definition or preorder)
rels     per relation: its name (a string), its width, its row count, where its types and columns start
types    a byte per column: text, node, int
cols     per relation, per column, its rows' values (i64), the rows sorted by their values, each once
bytes    the strings' bytes
ends     each string's end in `bytes`
docs     per document a node names: its key, its path (a string, or none), its first node (T's numbering, or
         none); by key. A document with neither is a guest's (0055)
places   per `@place` value a node column holds: the value, and how it is shown; by value
```

- **Flat and linear.** Each region is an array of fixed-width items. Every reference is an index into a region,
  so the image is read where its bytes are, with nothing decoded.
- **Of content.** A string is held as its text, once, and a text column holds its index in `strings`, not a
  process's atom id (0212). An image means the same in any process. (A `@place` value is still a process's
  numbering, `-2 - (path atom << 32 | line)`; its shown text is held beside it. Stage 2 makes it content.)
- **Sorted.** A relation's rows are in the order of their values, so two images are compared, and a delta is
  placed, by a merge.

### 3. Written by delta

**Stage 1: overlays of whole relations.** A session's run writes an overlay holding only the outputs it made again,
each whole. An output whose stratum was clean is read from the image below it, never read or written. Each image's
strings, documents and places are its own. A relation is read from the last image that holds it. Past eight
images, the run writes one base again, copying the outputs it did not make from the stack (`Builder::take`).

**Stage 4: overlays of rows.** Each relation entry has a kind: whole, rows added, or rows gone. A row is named by a
128-bit hash of what it shows (`Rel::hash`): a text by its bytes, an int by its value, a node by its document's path
and first node and its local part, a `@place` by its shown text, each cell with its type. So a row hashes the same
in every image and every process, whatever indexes its strings and documents have there. The session keeps, per
output directory, each relation's sorted row hashes as the stack reads it, with the stack's depth and the rows its
overlays changed (`session::Stack`). A run makes its outputs' image in memory and hashes each output's rows. An
output whose hashes are the same writes nothing. One that changed writes its gone rows' hashes (two int columns)
and its added rows, with the strings, documents and places they need. Where the change is at least half its rows,
it is written whole. A run that changed nothing writes nothing, so its file is as it was. A reader composes a
`View`: the last whole entry, less each later overlay's gone rows (by hash), plus its added rows. The stack is folded
into one base at eight images, or when its overlays changed more than an eighth of its rows.

The run's own deltas (`__minus`, `__plus`) were the first plan. They are not the outputs' change: an output is
gathered from the memo's file-local rows and the cross-file part's, filtered for loaded documents. Hashing the made
rows needs no second account of where each row came from. Its cost is one pass over the rows the run made, which
the run builds anyway.

### 4. Text is a reader

`Rel::lines` renders a relation's rows as the text lines the `.csv` files hold today, byte for byte. It serves the
readers that need text: the typing bridge's `path:local` join, `test-rr`'s `.expect` values, and the CLI's
diagnostics (each `rr_diagnostic` row a JSON string, a text column). The readers that need rows (phases, phase 0,
typing rounds, `typing::outputs`) read the columns.

## Stages

1. **The image, its writer and its readers** (`plan::output`, built): the format, a writer from rows or from the
   gathered lines, `Rel::col`/`row`/`lines`, whole-relation overlays and the fold. The corpus run writes the image,
   then makes each `.csv` file it wrote from it, so every reader and test reads the same bytes as before.
2. **Rows through the run** (built): `decoded_outputs` and the composed path hand each output as `Rows` (typed
   columns, values by row, each text once by content), shared (`Arc`) by the outcome, a session's kept outputs
   (`retain::Kept::output`) and the writer, never copied. The writer gathers each output's parts and puts them in
   the image as they are (`Builder::put`). `Rows::lines` gives the old text for the readers that still want it
   (memo entries, guests' requests, `typing::outputs`, tests through `Outcome::text`). Still text: a memo entry's
   file-local output (`memo::Entry::lines`), read as one part per output (`Rows::from_lines`), and the `.csv`
   files.
3. **The readers on the image** (built): `output::lines`, `text` (the bytes the `.csv` file held) and `rows` (each
   value as shown, a tab or a newline in it too) read a relation of the stack in a directory. Phases, phase 0, the
   typing bridge, constants, `test-rr`, the CLI's findings, a pack's members and the other dialects' findings read
   and write through them; the tests (about 180 reads in 22 files) read `output::text`. The `.csv` files are no
   longer written. The whole-run cutoff's record holds one base of the run's outputs (`output::base_of`; its key's
   version is 2). A session knows its stack by a stat of its file (length, modification time, depth: `session::Stack`),
   so an unchanged output is kept without reading the stack, and the stack is read only to fold it.
   `rerail-engine csv <out> [<to>]` writes the `.csv` files for the scripts that read them (the bench and pack
   scripts, the explorer, `sql-executor.py`, the data-layer probes), which call it after each run. Typing rounds'
   golden files and `rrlab` (Soufflé's own outputs) are not the engine's outputs, and stay.
4. **Overlays of rows** (§3, built): format 2 (an entry's kind), `Rel::hash`, `View`, `Builder::added`/`gone`,
   the corpus writer's overlay, the session's hashes.

## Measured

Stage 2 against the merged build before stage 1 (33fac9107), O125 replay steps 12–16, two rounds each, instructions
(profiling build, unprofiled run; both "same as full"):

| step | 12 | 13 | 14 | 15 | 16 | process |
|---|---|---|---|---|---|---|
| before, cross G | 11.73 | 6.35 | 3.51 | 4.16 | 4.53 | 142.6 |
| stage 2, cross G | 11.40 | 6.00 | 3.18 | 3.85 | 4.23 | 138.6 |

The baseline still had the `mentions` fault fixed in b62a05bc7 (some typing derivations differed). Sampled over the
whole replay (cold run included), the output code is now 260 of 4,912 samples: `decoded_outputs` 52 (117 before),
rendering the `.csv` lines from the image (`Rel::lines`) 149, putting the rows in the image 23. The `.csv` lines are
what is left: stage 3 moves the readers onto the image, and stage 4 writes only what changed.

Stage 3 (3cbb4cbb3) against the same baseline, two rounds each, on a loaded machine (load 40–90; instructions):

| step | 12 | 13 | 14 | 15 | 16 | process |
|---|---|---|---|---|---|---|
| before, cross G | 11.65 | 6.18 | 3.31 | 3.96 | 4.38 | 135.8 |
| stage 3, cross G | 11.15 | 5.80 | 2.96 | 3.66 | 4.04 | 128.3 |

Stage 4 against stage 3 is the same in instructions: cross within 0.1 G each step (11.43 / 6.05 / 3.18 / 3.82 / 4.22
against 11.44 / 5.97 / 3.17 / 3.83 / 4.23), process 135.3 against 134.5 G. Hashing each string and document once
per image (`Seen`), not each cell, brought the hashing from 0.75% of the replay's samples to 0.14%. What it changes
is what is written. Each step writes 33–482 KB, where its outputs whole are 2.3 MB:

| step | outputs unchanged | written by rows | rows written | bytes written | outputs whole |
|---|---|---|---|---|---|
| 12 | 7 of 18 | 10 (and 1 whole) | 24,434 | 481,968 | 2,321,264 |
| 13 | (folded: step 12 changed more than an eighth of the rows) | | | | |
| 14 | 8 | 10 | 1,592 | 32,960 | 2,324,832 |
| 15 | 9 | 9 | 5,348 | 107,088 | 2,324,960 |
| 16 | 7 | 11 | 5,518 | 101,104 | 2,325,696 |

Every run's outputs are a full run's ("same as full").

## Tests

- `an_image_holds_each_relations_rows_and_types_as_written`: every column type, no rows, no columns, 10,000 rows
  (sorted, each once).
- `an_images_lines_are_the_csv_writers`: against the writer's own rendering (copied as the oracle), for every
  combination of the two show flags: nodes by path and by definition, T's first nodes, guests, places, `unset`,
  JSON text, `007` in an int column, a node column holding `None`, rows of differing widths, rows of no columns.
- `an_overlay_replaces_the_relations_it_holds_and_the_rest_read_through`; a base written again starts the stack
  over.
- `a_folded_stack_shows_what_the_stack_showed`: its strings, documents and places copied.
- `rows_are_the_runs_lines_and_put_shows_what_the_writer_showed`: rows as `decoded_outputs` hands them (a text in
  an int column, a value holding a tab), their lines, lines read back, and the image `put` makes from two parts,
  against the writer's own rendering for every combination of the show flags.
- `rows_of_every_shape`: a node column holding `None`, a row of another width, no columns, untyped rows, `retain`
  and `without_first`.
- `a_damaged_image_is_refused`: magic, a region out of bounds, a text value naming no string, a stack that starts
  with an overlay.
- `a_directorys_outputs_are_read_from_its_stack`: `text`, `lines`, `rows` (a tab in a value), `names`,
  `put_lines` over a stack and folded at its depth, `base_of`, `to_csv`.
- `an_overlay_of_rows_gone_and_added_reads_as_the_relation_they_make`: hashes the same across images, a second
  overlay taking away a row the first added and one of the base's, the fold, an entry of rows gone refused in a
  base.
- `a_nodes_hash_is_its_documents_not_its_key`: one document under two keys hashes the same.
- `a_sessions_outputs_are_overlays_of_the_rows_an_edit_changed` (session): a run that changed nothing writes
  nothing; ten edits, each read equal to a fresh run's outputs, overlays holding rows gone, folded at the depth.
