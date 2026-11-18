# 0293. One tape API over every format, read in place

Status: decided (2026-11-16), by the user: "The tape needs to have a common, abstract API that abstracts away the
format behind it. We will remove some formats when we make a decision, but in the mean time it needs to be connected
up." Building. **0353 (2026-11-17):** every reader reads in place, so §6's `Projected::up` and the projection's
per-node arrays are copies, removed by 0353 step 2, `up` with them. Follows 0156 (format 16, draft) and the lab's `inplace::Reader` (t39).

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
   holds, as held. The lean parsers write the run's format directly (§7). A tape from another parser
   (tree-sitter's) is written from format 15's columns where a file's documents are made (`FileImage::single`,
   `FileImage::embedding`, after the embedders have read the host), in one pass to format 16 or straight to
   16z's blocks. A language without a table, or an image with a code its table lacks, keeps format 15, and the
   run says so once a language. The store keys a tape by the run's format
   (`tape_format_version`: 15, 16, or 1016 for 16z; `formats::stored_version`), so a store holds a document once
   per format and a run finds only its own. A restore checks the codes a tape uses (a language's table lists
   every code its grammar allows).

   Checked end to end (2026-11-16): the engine's workspace (207), syntax (35), matcher, findings, rule and wave
   (505) tests pass alike with `RERAIL_TAPE` 15, 16 and 16z, but for failures format 15 has too (pack data not
   pulled; Q22's two). The CLI under strict SQL gives byte-identical outputs in the three formats: 49 bandit and
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
     as it keeps `core` and `local`; the tape stores no parent. Kept on measurement (2026-11-16, the user: "drop
     the parent projections: yes, probably, but measure first"; `native/examples/kept_parents.rs`, profiling
     build, 50 Python files, 136k nodes, least of 5 rounds). One pass of every node's parent, as `sizes`,
     `depths`, `ordinals` and the granules each make, costs:

     | tape | walk (numbers, keeps parents) | a pass, kept | a pass, off the tape |
     |---|---|---|---|
     | 15 | 200 M instructions, 15.3 ms | 1.2 M, 0.23 ms | 120 M, 7.6 ms |
     | 16 | 453 M, 28.6 ms | 1.2 M, 0.23 ms | 54 M, 3.7 ms |
     | 16z | 463 M, 27.0 ms | 1.2 M, 0.22 ms | 71 M, 4.4 ms |

     Keeping them costs 4 bytes a projected node (0.5 MB here) and a push in the walk; each pass off the tape
     costs 45 (format 16) to 100 (format 15) times a pass over them. The same run shows the walk itself reading
     format 16 at 2.3 times format 15's instructions: a cost for the lab to weigh;
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

7. **The lean parsers write format 16 and 16z directly** (built, 2026-11-16, the user: "generalize the lean
   parsers so that they produce format 16 directly, and ideally format 16 compressed directly, without doing
   multiple passes or going through format 15"). Each lean parser crate's `tape_into` (the `.rr` parser's
   `lean::tape_into`, through the `Finish` trait, as it is generic over its widths) hands its writer, once the
   parse holds, to a finish the engine gives (`tape::formats::lean`, `formats::Rr`):

   - in a run of format 15, the writer's columns, as before (`Writer::finish_columns`, `TapeImage::written`);
   - in a run of 16 or 16z, each code the parser's keys name is numbered once in the language's table, and
     `Writer::finish_16` writes the tape from the writer's own working state (keys, starts, subtree sizes and
     tails) in one pass in preorder (`f16::write`): a node's slot is set when the next node is reached, and a block
     of 1,024 nodes is laid, or LZ4-compressed for 16z, as the pass leaves it. No format 15 column, image or format
     16 tape is made on the way to 16z.

   The writer's sizes and tails are exact by then, but where its widths were too narrow (the parser runs again at
   the wider widths, as for format 15); format 15's offsets are not read. A code the table lacks, or a node past
   format 16's bounds, keeps format 15 and says so once a language. `f16::write_from` (a tape of any format, read
   through the API) is the same pass, for tree-sitter's tapes (§4).

   An embedded program a lean parser writes may so be held in format 16 when its placeholders are cut out
   (`TapeImage::omitting`), which reads it through the API (it read format 15's columns at first, and stopped
   every Blade and Vue file in a run of 16).

   Checked (2026-11-16): for every lean parser whose language has a table (Python, JavaScript, TypeScript, TSX,
   Rust, PHP, SQL, Vue and `.rr`), the tape written directly in 16 and in 16z is byte for byte what format 15's
   image converts to, and answers every node's code, span, subtree and parent alike
   (`the_lean_parsers_write_each_format_directly`). The CLI under strict SQL (178 ruff rules over 10 files) gives
   byte-identical outputs in the three formats, cold and warm from each store; the stores hold 835 tapes in 1.20 MB
   (15), 727 KB (16) and 448 KB (16z).

8. **The matcher reads the tape** (step 5, its CPU half; built). A document (`TapeDoc`) holds no node record: the
   per-node `Rec` (42 bytes: kind, field, span, size, parent, first child, next and previous sibling, child count,
   flags), the alias overrides and the supertype list are gone. Its tape's reader is made once beside the image
   (`DocTape::reader`), and `AnyTape` answers a read at a time, dispatched on the format. A node's kind, field,
   flags, alias and classes are its code's entry in the table; its span, subtree and children are the tape's.

   - **ast-grep's node** (`TapeNode`, what the matchers walk) carries its parent where the step that reached it
     knew it: a child's, a walk's (`dfs`: the subtrees open around it), an ancestor's. A climb (`ancestors`) past
     the parent is one descent from the root (`Tape::path`: a step a sibling it passes, read from the subtree
     ends), the whole path at the cost of one. A previous sibling is read forward from the parent's first child;
     `prev_all` reads them once.
   - **A pass over every node** that wanted each node's parent takes it from the subtrees open around it
     (`TapeDoc::transducer_parents`): the transducer's packed batch (each node's children set their parent and
     next siblings), the relational syntax columns, the prefilter's statistics, the label passes' nesting, the
     set compiler's child and ancestor sets, a guest's extents (`node_extents`) and the anchors' index
     (`index_image`).
   - **Format 15's parent** searches back at most 64 nodes, then descends from the root over the siblings' subtree
     ends: never longer than the path's siblings, where the search back alone read every node of the earlier
     siblings' subtrees.

   Checked (2026-11-16): the tape, embedding, Vue, Blade, prefilter, anchor, facts, syntax, matcher, transducer,
   workspace, findings, wave, residency and statistics tests (1,065) pass in 15, 16 and 16z, but for four that need
   the packs' test data (the two pinned-capture tests and `definition_ids_are_pinned` pass with their fixtures
   restored from history) and, in 16 and 16z, the format 15 tape tests. The CLI under strict SQL gives
   byte-identical outputs in the three formats, cold and warm: the 23 Vue rules over 14 Blade, 9 Vue and 7
   TypeScript files (their programs embedded), and 178 ruff rules over 10 Python files.

9. **The device reads the tape** (step 5, its resident half; built). A resident document is its tape where it is
   placed, its source, its code table at a fixed width and its line starts: nothing per node beside the tape.
   - **Placed as stored:** format 15's image, format 16's tape, or a 16z tape as stored, read through its batch's
     cache of decoded blocks (decision 0353 step 4b; a block's four pieces decoded into a cache entry as a format 16
     tape of one block, decision 0426 §3). Until 2026-11-18 a 16z tape was laid out as the format 16 tape it is the
     blocks of (`Tape16Z::unblock_into`, then `decode_into` with 0336 §5's pieces decoded where they lie and 0336
     §8's on the GPU): a copy, now gone. `f16::placement` says where a format 16 tape's arrays, side lists
     and part starts lie, checked as `Tape16::from_bytes` checks them; the device's view (`DocView`) names them.
   - **The device's reads** (`metal_tape.metal`) are format 16's `View`'s (a value too wide for its array read from
     its side list; a start from its checkpoint and gaps), or format 15's columns. A parent is a search: format
     16's back over the siblings' slots, format 15's back over at most 64 nodes and then a descent from the root,
     as the CPU's. A child or a sibling is a hop over a subtree, bounded by the parent's end where the step knows
     it (a walk's child frame keeps its parent; a following-siblings frame its parent's end, found once).
   - **Gone:** the parent column, the child lists and the nodes by code the residency derived at admission
     (16 bytes a node). The interpreter's children are hops in both its resident and packed builds; its node
     record carries no parent or sibling links (`node_parent`, `node_next`). Where it pushed a node's later
     matching children last-first, it pushes them in order and reverses the run it pushed: the stack is the same.
   - **A pass's roots** were listed on the host from the tape's code bytes at first; §10 replaced the lists.

   Checked (2026-11-16): the device reads every node of a batch (one document of several side-list parts, with
   gaps and lengths past a byte) as its document does, in 15, 16 and 16z, admitted from a store
   (`the_device_reads_every_format_as_the_document_does`: code, parent, siblings, children, subtree, span); the
   combined graph and the rest of §8's set pass in the three formats, but for the same pack-data and format 15
   tests; of the slow tier (which the default filter leaves out, and an earlier note here wrongly counted as run),
   the micro walks equal the interpreter's labels, and with every walk spilling the device's roots equal the CPU's
   on all 491 clauses (41 waves over 24 of the pack's files, from 236fc98d2~1), in the three formats (the spill
   test's floor of 1,000 roots needs the packs' full test data: those files give 621); with `metal-tests`, the device's roots equal the
   CPU's on every clause the resident interpreter takes alone, in the three formats, but for one clause past its
   work limit, as at 6afcddb0a before this (work queue Q49, Q19 on oscar); the CLI's device run (`logic --metal`, the Vue rules
   over the mixed files) gives the same findings in the three formats, cold and warm.

10. **A pass's roots are a rule, not a list** (built, 2026-11-16, the user: "yes, please do that. It's a better
    shape"). §9's lists (4 bytes a root, a run's at the median 20 bytes a node, 85 KB a wave over 24 files, built
    on the host for every rule and run) are gone. The run's root table (`RootTable`, `root_mask` on the device)
    holds the batch's file count and each file's first group of 64 nodes, and per distinct rule: over codes, a
    bit per file it runs in and its test (kinds, then kind and class tests: the sweeps' `code_admits`); listed (a
    clause's prefilter candidates), a row of bits over the batch's nodes. Nothing is kept per root.
    - **A thread is a pass and a group of 64 of a document's nodes**: it reads the group's codes from the tape,
      makes the group's roots a 64-bit mask, and walks from each (`ROOTS_EACH`). The micro walks, the fused
      groups (a lane every `width`th group of its file), the pattern kernels and the resident interpreter's tasks
      (a task a group; the lane runs each root in it) all take their roots so. A group's roots run one after
      another in its thread: a long walk is still the spill queue's to split.
    - **The host lists nothing**: a rule is a few words, sized from the batch's node counts. The CPU's readers
      (`Roots::nodes`) still list a document's roots from the code bytes 64 at a time; the profile's per-depth
      root counts do so only when profiling.
    - **Not changed:** a wave's final root walks (`root_records`) take their starts from the wave's jobs, a list
      the wave's prefilter made.

    Checked (2026-11-16, on the build farm's M3 and M1 Max, 15, 16 and 16z): on the matcher's fixtures the
    device's roots equal the CPU's on all 491 clauses, the resident interpreter's on 114 of 122 with the D416
    clause past its work limit as before (Q49, Q19 on oscar); in 15 and 16, with the pack's files restored (618, from
    236fc98d2~1), the micro walks, the combined graph, the widened-walk catch, the device conformance test and the
    `waves::matches` tests pass, and with every walk spilling the device's roots equal the CPU's on all 491 clauses (the spill
    test's floor of 1,000 roots still wants the full test data). The two per-clause tests over all 618 files run
    past nextest's 240 s limit.

## Interim, until step 5

- **What reads the image's tape through the API** in whichever format it is held: the checks a restore makes
  (`tape::validated`), the embedders' reading of the host (`embed::host`), the axes (§5), and the image's node
  counts and subtree sizes.
- **What still copies it into nodes:** the transducer's packed batch (`PackedBatch::nodes`, 44 bytes a node, and
  its child lists, built from the tape in one pass): the roots' CPU JIT (`native_cpu`, a wave's roots run there,
  `waves::matches`), the grid and register machines and the non-resident interpreter read it. The resident path
  (§9) does not.
- **The core facts' digest** serializes a format 15 image to hash it (format 16's bytes are hashed as held).
- **The tape tests** (`tape::tests`) are format 15's. Since 0340 made format 16 the default, the tests that read
  format 15's columns pin their thread to format 15 (`formats::this_thread_stores`).

## Not done

- Format 16 tables for C#, CSS, HTML, Java, Markdown and Markdown's inline grammar: their lean parsers write
  format 15 until the lab's table generator (`tape16 table`) covers them.
- 16z's blocks over the language's dictionary: each block is compressed alone for now.
- The text stream (0156 §6), hashes per algorithm (§5), edits (§4).
