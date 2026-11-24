# 0656: A tape's auxiliary index: its names, vocabulary, places and counts, read in place in Rust and in UKL

Status: decided (the user, 2026-11-23: "If we took an interim "sidecar" before changing the tape format, what would it
look like in its most optimal form? … cheap to build, use little memory and buildable on a GPU via a ukl kernel, and
include the names stream idea, and include enough of the indexes that we could answer the rules that were active from
it. It should also be extensible …"; of the design page that answered it: "okay please go ahead make sure it's done in
a manner that we can read and look it up in both Rust and UKL"; and of its name: "please call it an auxiliary index,
not a sidecar").

## Intro

**Why.** Since t54, a batch makes rows to answer questions the tape cannot answer alone. The worst is `text` by a
string, about 26 s on PostHog: the relation is routed by node, so a lookup by string makes the whole chunk first. Next
come node rows, places and counts. Three of these are format changes that 0156 designed but never built:

- §6's names stream;
- a sibling ordinal on every 8th sibling;
- per-document code counts beside presence.

The rest need an index the format will never hold: text to nodes.

**What.** An auxiliary index is one per tape, a pure function of the tape and its source, kept beside the tape as presence is,
and read in place. The rules choose which sections a run reads, never which keys a section holds: the SQL index
prototype's rule, "never key postings to known rules".

## The decision

1. **The format** (`tape_format::aux_index`):
   - a header (magic, version, nodes, leaves, blocks, terms, the tape's digest);
   - a directory of sections (kind, version, parameters, offset, length);
   - each section 16-byte aligned, after four u32s of its own.

   A reader skips a kind it does not know, and answers `NONE` where a section is missing. The sections:
   - **LEAVES**: a leaf bitmap per 256-node block, and the leaves before each block. A node's leaf rank, and a
     rank's node.
   - **NAMES**: a byte per leaf, its term in the document, the most frequent first. 255 escapes to a per-256-leaf
     run of (leaf, term).
   - **VOCAB**: each term's 64-bit hash (`text_hash`) and its first occurrence's span in the source, so no text is
     copied. Then the terms in hash order.
   - **SIBLINGS** (version 2): every 8th *named* child of a family, as (parent, node). The places the engine reads
     are among named children (a projection's `idx`), so a place is the named siblings before a node, and the reads
     step over the tokens between.
   - **COUNTS** (version 2): (code, count), by code; and its *strays*, the named nodes under a parent that is not
     named. A projection of the named nodes leaves their subtrees out, so where there are none, the named codes'
     counts are its nodes'.
   - **RARE**: the nodes of each code with at most one node in 256, by code.
   - **NAMED**: the language's named codes, a bit a code, as the grammar says (a code's alias's word, else its
     kind's). The tape holds a code, not whether it is named; the builder is given them (`named_codes`).
   - **COMMENTS**: the language's comment codes (a kind named `comment`, its alias's name where it has one), a bit a
     code, and for each SIBLINGS entry the comments among its family's named children before it. T's shape numbers
     no comment, so its place is a place among the named children that are no comment (`place_past_comments`),
     read as a place is: the stored eighth's count less its comments, then at most 7 steps. The builder is given the
     code sets (`CodeSets`, `formats::code_sets`).
2. **One set of reads, written twice and held equal.**
   - Rust: `aux_index::AuxIndex`, plus `place` and `child` over a `Tape`.
   - UKL: `<ukl/reads/aux_index.h>` (`ukl_aux_*`) and `<ukl/reads/aux_index_tape.h>` (`ukl_aux_place`, `ukl_aux_child`,
     over the tape's `clif_tape_*`).

   `rerail-ukl-vec`'s `the_aux_indexes_ukl_reads_are_its_rust_reads` runs every UKL read on the CPU's vector path,
   interpreted and compiled, against the Rust read. It covers every node, leaf, term, code and section of a format
   16 tape with escaped terms, families up to 300 wide, and rare codes. The headers are in the UKL library, so every
   backend's preprocessing and the OpenCL gate check them.
3. **Not a copy (0353).** An auxiliary index is an index derived from the tape, written once where it is computed and loaded
   as its exact bytes. Nothing reads a row of it that the tape holds: a term is a span of the source, a place is
   ≤ 7 steps of the tape from a stored eighth.

## Steps

1. The format, the Rust builder (`aux_index::build`, over any `Tape`), the reads in Rust and UKL, and the tests that
   hold them equal. Done.
2. The builder in UKL (`rerail_gpu::aux_index::build`, `gpu/src/aux_index.uglykl`): one plan per batch of format 15
   or 16 tapes, with the same launches whatever the batch. Each kernel takes one element of the whole batch: a
   node, a leaf, a sorted row, a term, a code or a document. Between the kernels run the library's scans,
   compactions and stable radix sorts. The plan:
   - one pass over the nodes: leaves, codes and families;
   - one pass over the leaves: each text's hash;
   - the leaves sorted by (document, hash), which gives the terms;
   - the terms sorted by (document, count descending, first leaf), which gives their numbers;
   - the nodes sorted by (document, code), which gives the counts and the rare codes;
   - each document's layout, scanned into the output's offsets. This is the plan's one wait, to size the output;
   - each section written where it lies.

   The device refuses two kinds of batch, and the host builds those (`Refused`):
   - one where two texts of a document share a hash (each run of a hash is compared byte for byte);
   - one past the sorts' keys (65535 documents, or a code of 65536).

   Its bytes are `aux_index::build`'s: `the_device_builds_each_aux_index_as_the_crate_does`, on the CPU device and on
   Metal (an M1 Max). Done.

   The hash's device test found a fault in `<ukl/memory.h>`. Metal took a word's offset from the byte offset alone,
   and read a word through an unaligned base as it lay; the M1 Max read the wrong bytes. The offset is now the
   address's own on every backend, as CUDA's was, and is unchanged for an aligned base.
3. The store keeps the auxiliary index beside the tape, keyed by the tape's content and the sections' versions, and makes a
   missing section on demand. Done, as presence is kept (0434 §4): in the tape's home.
   - **Where.** After the presence, at its bytes rounded up to 16 from it (`f16::presence::aux_at`). A home is the
     tape, its presence, then its auxiliary index; it is stored, placed in a slab, admitted and loaded as one frame,
     so the host and the device read the one copy where it lies. The view the kernels read is full (32 words), so
     nothing in it names the index: `ukl_aux_in_home` finds it from the view's presence, as Rust does.
   - **Written once, where the tape is.** `TapeImage::written_16`, which every producer's tape passes through, builds
     every section (`lay_aux`) and lays them (`Written::lay_aux`). A tape without a presence gets none.
   - **Keyed by content.** The store keys a home by its source, language, grammar version and layout; the layout is
     8 (`F16_LAYOUT`), so a store of layout 7 reads cold, not without an index (9 since SIBLINGS' version 2). The index's header holds the source
     digest's first eight bytes; each section's version is in its directory entry.
   - **Read.** `Tape16::aux_bytes`, `Tape16Z::aux_bytes`, `TapeImage::aux_bytes`; `presence::find` checks the index's
     header and directory where it loads the home, and `presence_bytes` ends where the index starts.
   - **A missing section.** A home without one, or without a section a read needs, is the reads' abstraction's to
     make (step 4), once a document, where it is read.
   - **Tests:** `a_tapes_home_holds_its_aux_index_after_its_presence` (16 and 16z: the same tape, presence and index
     read back; laid once; a broken index refused), `a_stored_tape_holds_the_presence_its_writer_made` (a parse's
     index read back from the store), and `the_aux_indexes_ukl_reads_are_its_rust_reads`, whose kernel now reads the
     index in the tape's home through `ukl_aux_in_home`.
4. The engine reads from it: `text` by string and by node, a child's place, kind counts, and rare kinds. The rows
   these replace are removed, and so is the work that made them. In progress: the `text` rows are not yet removed
   (below, **Not yet removed**).
   - **One interface: `tape_format::doc_index::DocIndex`** (the user, 2026-11-23: "please put the calls behind an
     abstraction so that it's easier to switch out the design later"). The engine calls it and nothing under it:
     counts (`count`, `holds`, `counts`, `distinct`, `named_all_rooted`), listed codes (`listed`), text
     (`may_have_text`), terms (`term`, `terms`), places (`place`, `child`). Today every answer is the auxiliary index's; another design is a
     change to this file alone.
   - **Every home holds its index.** Every writer lays it (`written_16`; a tape without a presence is refused), and
     a home read in without one, or without a section the facade needs, is refused (`TapeImage::sixteen`), so read
     cold: layout 11. `FactsIn::index` reads it in place (`TapeImage::doc_index`); nothing builds one apart.
   - **Kind and class scans, rare kinds, the histogram.** `FactsIn::matching` (`of_kind`, `of_class`, `errors`)
     asks the index whether the document holds a code of the set, where it first scanned every code into a held
     list; where each code of the set is rare, its nodes are the index's list, with no scan. `histogram` is the
     index's counts, not a pass over the codes. The held list is removed.
   - **The `node` view's statistics** (`node_column_counts`): each document's nodes and codes are its index's
     counts where they answer for its projection (`Docs::indexed_codes`: core.rerail's shape, no anchors, no guest,
     a named root, no strays, not withheld), so the planner's first read of them makes no document. Else its
     projection, as before (T's shape leaves comments out, which the index does not count apart).
   - **`text` by its string.** A read of `text(n, s)` by `s` is routed (`MadeRows::route`, `keys_with_texts`) to
     the documents whose index says a node or span can have the text: a name of its vocabulary, else text its source
     holds. It made every document of the batch before.
   - **`text` by node.** A leaf's `text` row takes its string by its term in its tape's index (`DocIndex::term`,
     `core_rr::Terms`): the first leaf of a term interns its text, and every other leaf of the same text takes that
     string, with no hash of its text. Only where the home holds an index (`FactsIn::stored_index`): for a home
     without one, building it would cost what interning saves. A node that is no leaf interns its text as before.
   - **A child's place** (`child`'s `i`). A projection whose numbered nodes are every named node under the root
     (core.rerail's shape, one range, not withheld) over a home that holds an index reads a place in it
     (`Projected::places_by_index`, `FactsIn::place`), and its view entry says so (`ViewDoc::places_by_index`), so the
     views' reads call the CLIF read `tape_place` (`clif::reads::place`: the last stored every-8th named sibling,
     then at most 7 named steps of the tape), which the host's JIT, the CPU's vector path and every device read the
     same. T's shape reads its places past comments (`tape_place_past_comments`, `FactsIn::place_past_comments`).
     An anchored projection's outermost anchors hang from the root, whose children are the anchors: an anchor's
     place is its rank among them, a rank item of the projection's numbering (`Item::Anchor`, `ANCHOR_ITEM`), read by
     the host and the device alike. The places column is removed, with its run in the views' entry (`ViewDoc::place`
     says which read a node's place is).
   - **Not yet removed.**
     - *The `text` rows*: still made a row a node, now with a leaf's string by its term. Removing them makes `text` a
       view, and how its `s` is valued decides that view for every node, leaves included: a span (compared by its
       bytes through the vocabulary's hash, interned only where a rule outputs it), an interned string (multi-leaf
       texts, which have no term, interned into a table by node: the row again), or a term for every node with text
       (an index of about 20 bytes a node). The user's to choose.
   - **The reads take the home.** `DocTape::in_place` hands the reads the home's bytes (`extent`): the tape, its
     presence and its index, which the view names (`presence`). A home ends where its index ends
     (`AuxIndex::extent`; `presence::find` refuses bytes past it).
   - **Tests:** `a_documents_index_answers_as_the_document_says` (the facade, on a home's index and a built one; one
     without a section refused), `a_read_of_text_by_its_string_makes_only_the_documents_that_can_hold_it`,
     `a_node_columns_counts_make_no_document`, `the_clif_read_of_a_place_is_the_rust_read` (`tape_place` through the
     JIT, every node), `the_view_reads_are_the_hosts_rows` (the view reads on the JIT and a device, core.rerail's
     shape now reading places in the index), and the facts' tests of every format
     (`every_format_reads_the_same_facts_off_the_tape`).
