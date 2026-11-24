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
   these replace are removed, and so is the work that made them.
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
   - **`text` is a view** (`View::Text`), in core.rerail's shape: no row is made, and no text interned until a rule
     reads it. Its rows, a document's in order: the numbered nodes with text, a rank item the projection's walk
     marks (`Item::Text`, `core_rr::TextRows`: a node like one whose text a rule reads, `Projection::text_nodes`,
     within the inline bound or token-like or of a kind read at any length, as the rows were), then the positions
     with text (`TextSpans`: the tokens where a rule reads a token's text, the source and the lines where one reads
     theirs). The walk that made a row a node, and its decoding of literals to decide a node's text, are removed;
     so is `RERAIL_LINE_TEXT=all`, which made every line's text within the bound.
     - *`s`* is the text's atom (`Projected::text_atom`): a leaf's by its term in its tape's index
       (`DocIndex::term`), interned at the term's first read and read after with no hash of its text (the term's
       atom, in the projection's block: a derived fact written once, decision 0353); any other's interned where it is
       read, as `op`'s are.
     - *By node* (`n`): the node's or position's row, by the rank (`Projected::text_row_of`).
     - *By string* (`s`): the rows of the documents whose index says a node or span can have the text (a name of its
       vocabulary, else text its source holds; `Batch::text_matches`), each one's rows compared by their bytes. A run
       gives a batch its documents' counts (`ViewCounts`' 11th: `Projected::text_rows`), so a read makes only those
       documents; with no counts given, the view's row numbering makes every document's projection (not its rows).
     - *`Extra`'s spans* (the source, the lines and the regexes' matches, where a program reads `source`,
       `src_line` or `rx_span`) have their text made with them: a relation of those rows alone, bound after the view's
       (`places::join`, as their places are). Not counted by value (`place_statistics`): that would intern every text.
     - The device reads no `text` (`device_chunk::device_view`), as it reads no `op`.
     - *An atom, not a span* (the user, 2026-11-24, of the trade-offs: "that's okay, leave it as atoms"). A span would
       intern nothing until an output and let the device read `text` in place, but it is a new kind of value in every
       join, dedup, sort, SQL read and statistic of a string, and a derived fact holding one holds its tape.
       `text_atom` and `text_matches` hide how an atom is made and a string is matched, not the column's type: spans
       would change its readers too.
     - *A text test's scan* (`contains("ol", x)` over a batch's strings, `Tested::scan`) tests the view's texts too,
       a document's at a time: they are no group's strings. A whole scan made every document already; it interns
       what the rows interned, and only there.
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
   - **The reads take the home.** `DocTape::in_place` hands the reads the home's bytes (`extent`): the tape, its
     presence and its index, which the view names (`presence`). A home ends where its index ends
     (`AuxIndex::extent`; `presence::find` refuses bytes past it).
   - **Tests:** `a_documents_index_answers_as_the_document_says` (the facade, on a home's index and a built one; one
     without a section refused), `text_is_a_view_of_the_nodes_and_positions_with_text` (no row made; a row a node
     with text and none past the bound, a position's, by node and by string; the leaves of one text one atom;
     narrowed to the kinds read; `Extra`'s source span after the view's), `a_probe_makes_the_document_it_names_alone`,
     `a_read_of_text_by_its_string_makes_only_the_documents_that_can_hold_it`,
     `a_node_columns_counts_make_no_document`, `the_clif_read_of_a_place_is_the_rust_read` (`tape_place` through the
     JIT, every node), `the_view_reads_are_the_hosts_rows` (the view reads on the JIT and a device, core.rerail's
     shape now reading places in the index), and the facts' tests of every format
     (`every_format_reads_the_same_facts_off_the_tape`).

## Follow-up: steps 3 and 4 on PostHog, profiled (2026-11-24)

A cold profiled PostHog run (the M3, base b8e270700a against 827049b60e) gave the same findings (29,965) at about
the same CPU. Under that total, four costs had moved or grown. Each was profiled (the engine's spans and macOS
`sample`) and cut where the cost was work done twice; base and head were then run in turns on the same host, since
its load swung from 49 to 9 within a pair.

| Own CPU (s), in turns | base | 827049b60e | f31f7cae0e |
|---|---|---|---|
| The projections' identity maps | 9.7 | 16.3 | 10.8 |
| `literal_str`'s joins, own | 1.4 | 7.1 | 8.0 (fedd76a1a8 below) |
| The tape's parse, own, and the index's build | 19.5 | 25.2 | 19.9 + 2.5 |
| The node rows (`text`'s rows no longer made) | 17.8 | 2.5 | 2.9 |

f31f7cae0e retired 3,645–3,651 G instructions against base's 3,876 G (−6%), with the same findings.

- **The index's build at the parse** (`aux_index::build`). Each code was read three times, the leaves listed
  twice, counts kept in a tree, the names' map hashed with SipHash, and the grammar's code sets
  (`formats::code_sets`) made at every document's parse. Now each code is read once, the leaves listed once, counts
  are a vector by code, names an FxHashMap, and a grammar's code sets are made with the grammar
  (`Grammar::aux_codes`). The bytes are the same (the tape digest pin and the GPU builder's test). What remains,
  2.5 s, is the index itself: a term's lookup, the leaf scan and the position reads.
- **The index read at every atom.** `TapeImage::doc_index` parsed the index's header and directory at each call, and
  `text_atom` called it for every atom. An image now reads them once when it is made (`IndexAt`, `DocIndex::of`).
- **The identity maps.** `text`'s mark in the projection's walk (`TextRows::has`) spent 2,472 of its 2,709 samples in
  its gate (`NodesOf::reads_coded`), which read again the node's field, its parent, the parent's kind and field, and
  its text (twice). The walk's path now carries each open node's code and field, and the gate takes them from it
  (`NodesOf::reads_known`), with the text read once. What remains (+1.1 s) is the length and string tests of the
  nodes the gate passes, which the made rows did too.
- **The statistics pass** did not build projections twice, as first reported. The planner's counts of the `node`
  view (`doc_codes`) were read off the projection of a document already made, every node's kind each time; they are
  now read off the index, made or not.
- **`literal_str`.** The cost moved there from the made rows was not interning by the rows' reads but a text test's
  scan of the view (`Tested::scan`): it interned every text of the batch one at a time (`Atoms::intern`: a read lock,
  and at a miss two vectors and the write lock, a sixth of the samples waiting on the lock), and copied each. It now
  interns a document's texts together (`Projected::text_atoms`, `Atoms::intern_all`) and tests them where they lie
  (fedd76a1a8). A first change that interned only the texts passing a test was slower (the lookups then interned the
  rest, and the scan tested a common text again in every document) and was not kept.
- **Not measured yet:** fedd76a1a8 on PostHog. Its runs on the M3 were killed by the farm's GPU watchdog (the GPU
  stalled from 11:20, with no farm program on it); the engine holds a Metal device in a CPU run too.
