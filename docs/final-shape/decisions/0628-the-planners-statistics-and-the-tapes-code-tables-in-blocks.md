# 0628: The planner's statistics are overlay blocks; a tape's code table is a view of bytes

Date: 2026-11-21. Status: decided (the user, 2026-11-21), being built.

Numbered 0569 where it was written (Two-pass-roots-and-cross-file); renumbered 0628 in the merge of the
serverplan-quality hub, whose 0569 (`clif_rust` is removed) every other worktree cites.

The user's order: every run data structure, the planner's and the program image's, moves into blocks, and the Rust
ones go ("NONE of these should exist. These are CPU-only structures and break the invariants that the CPU and GPU
run on the same data"). The profiler and third-party code stay on the heap. Two of those structures needed a design
of their own, and the user chose each.

## 1. The planner's statistics: one block table each, a fork an overlay

`plan::table`'s `RelMap` and `ColMap` (`Model::ndv`, `Stats::fan`, `dfreq`, `heavy`, `keys`, `counts`, the
components' `RelMark`) were copy-on-write: a fork (`Model::fork`) shared each relation's column vector by a count,
and a write copied that relation's (decisions 0209 §3, 0210 §9).

- Each statistic is one block table keyed by (relation number, column).
- A fork is an overlay block over its parent's, as a program image's overlay is over its parent (0241). A read
  falls through to the parent where the overlay has no entry, and a removal is an entry that says so. A fork costs
  what it writes, not what it shares, and nothing is copied.
- A value that is not plain data becomes a Pod record in a block arena, named by its index: a heavy column's
  values (`HeavyCol`), a join's counts (`JoinCounts`), a bloom filter's keys (`bloom::Keys`), and a column's
  constants' counts (`HashMap<Constant, i64>`).

## 2. A tape's code table is a view of its bytes

`tape_format` sits below `rerail-blocks` (blocks → device → tape_format), so its types cannot hold blocks. Its
`CodeTable` (a `Vec<Entry>`, each with a `String` alias and a `Vec<u16>` of classes), its index of `CodeSet`s, and
format 16's `Lang` (the table again, and a `HashMap<Entry, u32>`) were Rust structures.

- `CodeTable<'a>` and `CodeSet` are views over an encoded table: fixed records, then the aliases' bytes, then the
  classes. `entry(n)` gives an `EntryRef<'a>` (kind, field, flags, the alias as `&str`, the classes as `&[u16]`).
- Format 15 reads its table in place, from its image's bytes.
- Format 16's `Lang` encodes its table once, when it is loaded, into one byte buffer. The engine places that
  buffer in a block, and every document of the language reads it there.

## 3. The plain arrays sit below the tape format; a tape's writer writes in blocks

The user's choice, 2026-11-21: "Vector below the tape". The writer's columns (`Writer`'s code, size and tail
columns, its starts, keys, code ids, marks, chains and held wraps; format 16's parts; the compiled tier 2's
buffers; simdjson's columns) were `Vec`s, and `finish` handed the engine a `Vec<u8>` that it then kept or copied.

- `rerail-vector`, a crate below `tape_format`, holds `Pod`, `Region`, `Room` (a region of a size and alignment,
  `Room::region`) and `Vector`, the growable array over a room's regions. `rerail_blocks` re-exports them
  unchanged, so there is one `Vector` and one `Pod` in the workspace.
- The engine installs the blocks' room at load (`rerail_blocks::maps::install`, which calls
  `rerail_vector::install_default_room`). A `Vector` made with no room takes its regions from it: the run's open
  blocks. A crate's own tests, run without the engine, take theirs from the heap (`HeapRoom`).
- Every writer column is a `Vector`. `Writer::finish` returns the image as a `Vector<u8>`, `finish_columns`
  its columns as `Vector`s, and a tape's own home (`home::Home::Own`) holds that `Vector`: the bytes the writer
  wrote are the bytes every reader reads, in the block they were written in. The code column of
  `finish_columns` is moved down over the header in its own region, not copied out.
- The code-id tables (`Writer::id_of`, tier 2's `Tape::id_of`) were thread-local `Vec`s kept between tapes, so
  that a tape cleared only the ids it set. A thread-local would hold a run's region past its run, so each tape
  now has its own table in the run's room, filled with `NO_CODE` when it is made.

## 4. Plans and program analyses are sections of the program image

The user's choice, 2026-11-21: "Image sections". The planner's outputs and the program's analyses (a rule's
`JoinSpec` and its `JoinRule` steps, `Shapes`, the components of `components_of_graph`, and the image's own
`Parts`: its declarations' and facts' maps, a fact relation's rows of cells) were trees of Rust enums, strings and
nested vectors.

- Each is encoded once, where it is computed, into a section of the program image: fixed records, then their
  lists and their text, in the image's words. The image is in blocks already, and a fork's image is an overlay of
  its parent's (0241), so a plan made in a fork costs what it writes.
- Each is read through a view over its section's words, as a tape's code table is (§2): no reader rebuilds the
  tree.
- The image's maps keyed by name become sorted record sections, found by a search of their keys.
- The user's choice, 2026-11-21: "In-house derive". One derive macro (`rerail-section-derive`, with its runtime in
  `rerail-section`) writes each tree type's section encoder and its archived view: a fixed `repr(C)` record per
  struct, a `repr(C)` tagged record per enum, and lists and text out of line, named by offsets relative to the
  field that names them, so a section reads the same wherever its block maps it. A reader matches on the view
  (`ArchivedStep`) as it matched on the type. No third-party format sits in the image.
- A field that holds a run's object (a bound relation, `Arc<InRel>`) is not part of a plan: the plan names the
  relation by its number, and the run binds the numbers to its relations when it executes the plan.

## 5. Nothing at store time reads a tape again: what the writers know, they write

The user's question, 2026-11-21: "Don't we calculate all of the store-time stuff when we parse? Why are we
re-calculating it?", and the proposal the user had built (2026-11-22). Three passes read a written tape again: the
syntax errors' scan, the guests' extents, and the walk that left an embedded program's placeholders out.

- **A tape's faults are listed by its writer.** As a block is made final, each writer (`writer16.rs`, the CLIF
  ops, the device's `ukl/writer.h`, the reference) sets the block's fault bit and lists each ERROR or MISSING
  node's index in its ERRORS part, beside the fault bit. `finish` lays the list, in preorder, after the presence in
  the tape's home (`f16::presence::faults`, the presence header's fourth word its count; `F16_LAYOUT` 7).
  `TapeDoc::syntax_error_nodes` and `FactsIn::errors` read the list there; the first reads only the outermost
  faults' blocks, and `has_syntax_error` is one read of the fault bit. Format 15's tapes keep the scan of the
  blocks their presence flags.
- **An embedded program's placeholders are left out as it is written.** The embedders read their programs with
  `Parse::read_program`, which hands the lean parser's writer the omitted ranges (`Writer::omitting`), as
  `encode_tree_image` already left a tree's out. The second walk (`TapeImage::omitting`) is gone.
- **A guest node's extent is read from its span, never stored.** It is where the node is placed in the host
  (`Origin::of_node`, `bounds`): from the first of its tokens that stands for host bytes to the last
  (`SegmentMap::stands_for`), its tokens its stored leaves and the runs of text that are not white space in the gaps
  between them (the implied tokens, RER-131), read off the tape from each end until one stands (`origin::extent`): the
  rule the stored column was written by, so a root's span, which holds the white space after its last token, and a
  marker its embedder wrote over no host byte are not its place. The user's choice
  (2026-11-22), after measuring the stored column at 8 bytes a node, 43% of the guests' tapes, with 6.7% of nodes'
  extents differing from their spans: "Can't exceptions trivially be identified from the node?". The column
  (`file_documents.extents`, now a `RETIRED_COLUMNS` entry dropped at open), `extents_of`'s pass and the restored
  vector are gone.

## 6. A 16z tape is read through its batch's cache

The user's choices, 2026-11-21: the cache is the one "which owns the GPU version", taken "Per read".

- A batch (`facts::vtab::syntax`'s `Growth`) owns one `f16::ZCache`, in the device's buffer where there is a
  device. Every reader of its documents decodes a block into it: the host's reads (`F16Z::with`), the JIT's and the
  CPU transducer's (`ZCache::view_for`, the shared cache's checked reads), and the kernels'. The per-reader caches
  (`view::Reader`, the thread-local slots) are gone.
- A document holds no cache. A read takes its batch's (`FactsIn`, `TapeDoc::in_batch`); a reader outside every
  batch (a host read by an embedder, a store-time read) makes a batch of the documents it reads
  (`TapeImage::own_cache`, `Facts::alone`).
- A batch's cache is sized by the documents made when it grows. A document made when first read (`syntax::Pending`,
  Q332, from the merge of django-bugs, 2026-11-22) has no tape until then, so its blocks may not fit the batch's
  entries. Such a tape is read in a batch of its own (`TapeDoc::in_batch`, `Projected::narrowed`), and
  `Tape16Z::reader` refuses a cache whose entries are smaller than its blocks. A misfit is never read wrong.
- A 16z tape is not walked when it is loaded: its reads are bounded (0463), so a malicious tape cannot reach
  memory outside its block. The user's rule: "The requirement is not that we validate every tape on every
  decompression, it's to stop a malicious tape from reading or writing memory it shouldn't".

## 7. A `.rerail` file is read as every document is

The user's rule, 2026-11-22: "I do hate the idea of having many data structures representing the same thing and
many ways to solve the same problem"; then, of moving rule files onto the documents' tape: "do it". The user's
choice for its 16z reads' cost: "Faster 16z reads".

- A rule file (`rr::from_tape::RrFile`) holds a `DocTape`, as a document does: the tape `TapeImage::encode_rr`
  writes in the run's format (the one the engine's documents of `.rerail` files have), its reader made once, and in
  16z its batch's cache. A file is kept for the process by itself (`rr::read_shared`), so it is read in a batch of
  its own (`TapeImage::own_cache`). Its reads go through the tape API (`tape::Tape`): a code's kind, field and flags
  from the table, a child's code and subtree end in one read (`Tape::code_and_end`), a span in one (`Tape::span`), a
  line from the tape's newline index (`F16::line`, `F16Z::line`; format 15 holds none, so a run that stores it counts
  the newlines). An ERROR is looked for among the faults the writer listed (§5).
- Gone: format 15's own reader of a rule file (`tape_format::View` over `rerail_rr_syntax::tape`'s image), the code
  keys it decoded into a column of their own, and the line starts (`syntax::Lines`) a read made. `rr::message_masked`
  reads the same tape (`RrFile::parsed`). The format 15 writer stays the parser's own target and its oracle tests'.
- A 16z read probes its batch's shared cache, a lookup of several atomics and fences, and a rule file's reader reads a
  node's fields one after another. A 16z tape keeps the entry its last read found (`Hint`: the entry's address and
  completion epoch), and a read of the same block reads it again with no lookup while it still holds the block
  (`cache::still`: its generation not turned, its key and completion unchanged), checked again after the read, as a
  lookup's readers check. `a_hinted_read_answers_while_the_cache_turns` reads through a cache of two entries a
  generation from four threads.
- What stays a list: a program's statements (`Vec<Stmt>`, `rr::Files`), which name the nodes of its files it reads,
  in its order, includes resolved. They are a program's choice across files, not one file's nodes.
