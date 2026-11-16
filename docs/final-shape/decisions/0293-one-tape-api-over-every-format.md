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

This record is steps 2 and 3, and the API they need.

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
     nodes, and reads it in place as format 16 is read; the reader keeps the last two blocks it decoded.

   `AnyTape` holds one of them and dispatches once a document (`with_tape!`), so the per-node reads are
   monomorphised. A format the lab drops is removed from `AnyTape`; a candidate it adds goes in the same way.

3. **Format 16's choices ship with the engine.** A language's code table (every code its grammar allows, numbered
   most frequent first), its cutoff, its widths and its revision are a table the lab writes (`tape16 table`) and the
   crate embeds. A language without one has no format 16 tape: format 15 serves it, and the run says so.

4. **The format a run reads** is `RERAIL_TAPE`: `15` (the default until 0156 is accepted), `16`, or `16z` (the fixed
   blocks); another value stops the run. A document's `AnyTape` comes from its `TapeDoc` (`TapeDoc::tape`): format
   15 over the image's own columns, with the image's code table as the API's (made once an image). A language
   without a table, or an image with a code its table lacks, is read as format 15, and the run says so once a
   language.

5. **The axes read the tape** (step 3). `within.rs`'s kind-set axes (`nearest`, `inside`, `has`) take membership
   from the code, and climbs, subtrees and the batch's solve from the tape's structure (the lab's `prim`: nearest
   by a bounded climb, outermost by a prefix max, has from the subtree). Answers are mapped to `Projected`'s
   locals, which are a subset of the preorder ids in order.

## Interim, until steps 4 and 5

- **A run reading format 16 also holds format 15.** `Facts`, `TapeDoc`'s records, the matcher and the Metal kernels
  read format 15 until they move to the API. So no memory or time is measured before then.
- **The writer is driven from format 15's finished columns** where a tape is written, not by the parser: the
  parsers' format-15 writer wraps and rewinds (`Writer::insert`, `rewind`), which format 16's writer has no
  calls for yet. Format 16 is to be stored beside format 15 and read back as stored, not written again at each
  load. Until the store keeps it, a document's format 16 (or 16z) is written from format 15 once an image, the
  first time the run reads it in that format (`tape/formats.rs`). The store must keep it before anything is
  measured: a load-time write would be counted as a read.
- **The axes' Metal solve** still climbs a parent array on the device (`facts::nearest`). The lab's format-16
  kernels (E30, E31) replace it in step 5.

## Not done

- The writer's calls from every parser (0156 §7), and the store's format-16-only rows.
- 16z's blocks over the language's dictionary: each block is compressed alone for now.
- The text stream (0156 §6), hashes per algorithm (§5), edits (§4).
