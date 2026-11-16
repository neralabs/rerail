# 0293. One tape API over every format, read in place

Status: decided (2026-11-16), by the user: "The tape needs to have a common, abstract API that abstracts away the
format behind it. We will remove some formats when we make a decision, but in the mean time it needs to be connected
up." Building. Follows 0156 (format 16, draft) and the lab's `inplace::Reader` (t39).

## Intro

A tape is one file's parse tree as bytes. Today the engine reads format 15 only, and never in place where rules
read it:

- `TapeDoc` turns the image into a 42-byte record a node (`tape::Rec`), and `Facts` turns those into per-node
  columns, child lists and per-kind postings (`facts/core.rs`).
- The rules' views and axes (`facts/vtab/syntax.rs`, `within.rs`) read `Facts`; the matcher and the Metal kernels
  read `TapeDoc`'s records.

Format 16 (0156) is built and measured in the lab only. The user's terms for the engine (2026-11-16):

- The kernels operate over the tape directly, compressed or decompressed, format 15 or 16. There is no
  "decompress to an array of nodes" step. What that choice costs is what the lab is for.
- So nothing is measured until five steps are done: 0156 accepted or amended; format 16's writer and reader in the
  engine; the axes over the reader; the base views on the tape with `Facts` gone; the matcher and Metal on it.

This record is steps 2, 3 and 4, and the API they need.

## Decision

1. **One API, in the format crate** (`rerail-tape-format`, module `tape`). A `Tape` is one document's tree, read
   where it lies. Node ids are preorder numbers, `0..len`. It answers:

   - **the code**, as a number in the language's code table (`CodeTable`): kind, field, flags, alias and supertype
     classes come from the table, so a kind test is a number test;
   - **structure**: subtree end, first child, next sibling, parent;
   - **spans**: start and end at a point, and a cursor's step in preorder;
   - **in bulk**: a code set tested 64 nodes at a time, and every span in order.

   It is the lab's `Reader`, as the lab measured every candidate through it.

2. **Every format behind it**, each read in place:

   - **format 15**: the image's columns (`code`, `size`, `base`, `off`, `tail`) at their widths. The tape's own code
     table is mapped to the language's once a tape (one entry a code the tape uses, not a node);
   - **format 16**: the writer's five arrays and their side-list parts (0156 §1);
   - **format 16 in fixed blocks** (0156 §2.1, `16z`): LZ4 blocks of 1,024 nodes, with the block index. A block is
     one side-list part, so a read's arrays and side lists are all in its block (only a parent's sibling steps
     cross back). A read decodes the one block it falls in, to the tape's own bytes for it, never to an array of
     nodes, and reads it in place as format 16 is read; each thread keeps the last two blocks it decoded, keyed by
     tape, so a reader is made per read and decodes nothing again.

   `AnyTape` holds one of them and dispatches once a document (`with_tape!`), so the per-node reads are
   monomorphised. A format the lab drops is removed from `AnyTape`; a candidate it adds goes in the same way.

3. **Format 16's choices ship with the engine.** A language's code table (every code its grammar allows, numbered
   most frequent first), its cutoff, its widths and its revision are a table the lab writes (`tape16 table`) and the
   crate embeds. A language without one has no format 16 tape: format 15 serves it, and the run says so.

4. **The format a run stores and reads** is `RERAIL_TAPE`: `15` (the default until 0156 is accepted), `16`, or
   `16z` (the fixed blocks); another value stops the run. The store keeps one format, the run's: a document's
   image (`TapeImage`) holds format 15's columns or format 16's (or 16z's) bytes as stored, and its tape
   (`TapeImage::tape`) is read where it lies in that format. Each stored form starts with its own magic
   (`TMPTAP9N`, `TMPTAP16`, `TMPTAP6Z`) and the language's tag after it, so a load reads whatever the store
   holds, as held. Format 16 is written from format 15's columns where a file's documents are made
   (`FileImage::single`, `FileImage::embedding`, after the embedders have read the host): the parsers write
   format 15 until they call format 16's writer. A language without a table, or an image with a code its table
   lacks, keeps format 15, and the run says so once a language. The store keys a tape by the run's format
   (`tape_format_version`: 15, 16, or 1016 for 16z; `formats::stored_version`), so a store holds a document once
   per format and a run finds only its own. A restore checks the codes a tape uses (a language's table lists
   every code its grammar allows).

   Checked end to end (2026-11-16): the engine's workspace (207), syntax (35), matcher, findings, rule and wave
   (505) tests pass alike with `RERAIL_TAPE` 15, 16 and 16z, but for failures format 15 has too (pack data not
   pulled; Q17's two). The CLI under strict SQL gives byte-identical outputs in the three formats: 49 bandit and
   178 ruff rules over 10 files, cold and warm, and three rules over a file written to trigger them, read from
   each store as stored (no tape written again). The stores hold 322 KB of tapes in format 15, 173 KB in 16 and
   110 KB in 16z (raw, before the store's zstd), for 68 tapes.

5. **The axes read the tape** (step 3; built). A document's facts hold the tape they were read off (`Facts::tape`,
   the image moved in, not copied; the same `DocTape` its `TapeDoc` holds), and its bytes count in a chunk's
   resident cost. `within.rs`'s kind-set axes (`nearest`, `inside`, `has`) take membership
   from the code, and climbs, subtrees and the batch's solve from the tape's structure (the lab's `prim`: nearest
   by a bounded climb, outermost by a prefix max, has from the subtree). Answers are mapped to `Projected`'s
   locals, which are a subset of the preorder ids in order: a node the projection leaves out is skipped on a climb
   and in a subtree, and is never a member. `within`'s tree containment reads the tape the same way (every code
   is in its set). The batch's CPU solve is one pass of the set's matches, 64 codes at a time, reading only a
   match's subtree end (`tape::nearest_all`, the lab's `axis_groups`): no parent is read. A climb per looked-up
   node reads the format's parent: a few sibling steps on format 16, a search back over the nodes before on format
   15 (the cost 0156 §1.7 names), which is what the lab weighs.

6. **The base views read the tape** (step 4; built). `Facts` holds a document's tape (`DocTape`, shared with its
   `TapeDoc` where the matchers keep one), its source, lines and grammar, and no column: a node's kind, flags,
   field, span, subtree end, parent and children are the tape's answers through the API, in the format it is held
   in. The nodes of a kind or a class, the kind histogram and the ERROR and MISSING nodes are scans of the codes, 64
   nodes at a time (`match_groups`), made where asked; nothing is posted. A text-only document has no tape and no
   nodes. A restore checks the image (`validated::visit`) and moves it in.

   A walk reads parents from the subtrees open around it (`Facts::with_parents`: one subtree end a node), so it
   reads one parent off the tape a range, not one a node; on format 15 a parent read is a search back. So does
   everything that asked for parents node by node:

   - a projection keeps the parent of each node it numbers (`Projected::up`), found in the walk that numbers them,
     as it keeps `core` and `local`; the tape stores no parent;
   - a Python call's execution context is the innermost body that holds it, from the bodies' ranges
     (`families::Contexts`), not a climb of the call's parents;
   - "an ERROR or MISSING node around this one" is a test against those few nodes' subtrees
     (`Facts::under_error`, `touches_error`), not a climb.

   The core facts' digest is of the source and the tape's stored bytes (`core-facts/v2`). Checked (2026-11-16):
   a test reads every node's answers and every kind's and class's nodes alike in the three formats, for twelve
   dialects; the facts, syntax, Python producer, workspace, findings, rule, wave and matcher tests (849) pass in
   15, 16 and 16z, but for the failures format 15 has too; the CLI under strict SQL gives byte-identical outputs
   in the three formats (49 bandit rules and 178 ruff rules over 10 files, cold and warm; the bait file's five
   findings).

## Interim, until step 5

- **What reads the image's tape through the API** in whichever format it is held: the checks a restore makes
  (`tape::validated`), the embedders' reading of the host (`embed::host`), the axes (§5), and the image's node
  counts and subtree sizes.
- **What still copies it into nodes:** a document's records (`TapeDoc`'s `Rec`), built from it at restore,
  through the API, in every format, for the matcher. Step 5 removes them. So nothing is measured before then: a
  restore's copy would be counted as a read.
- **The core facts' digest** serializes a format 15 image to hash it (format 16's bytes are hashed as held).
- **The device's kernels read format 15 only** (`facts::nearest`'s Metal solve, the matcher's Metal runs): a
  Metal run over a store of format 16 stops at the residency with the reason (`resident::header_counts`). The
  lab's format-16 kernels (E30, E31) replace them in step 5.
- **The tape tests** (`tape::tests`) are format 15's: they assert the run stores format 15, and fail under
  `RERAIL_TAPE=16`.

## Not done

- The writer's calls from every parser (0156 §7).
- 16z's blocks over the language's dictionary: each block is compressed alone for now.
- The text stream (0156 §6), hashes per algorithm (§5), edits (§4).
