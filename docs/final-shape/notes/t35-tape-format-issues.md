# T35: what the tape costs to write and to read (a survey before a new format)

## Intro

A tape holds one file's parse tree as columns. The format is v9np, format 15. It is not
deployed anywhere yet, so this is the time to change it. This note lists the problems I found in
writing a tape and in using one, surveyed across the hub and its sibling worktrees on 2026-11-13.
It proposes no new format.

```text
header   magic, language, three widths, version, source length, five lengths   64 B
code     per node: an id into this tape's code table, by first use              2 B
size     per node: its subtree, itself counted                                  2|4 B
base     per 16 nodes: the least start in the block                             4 B
off      per node: start - base                                                 2|4 B
tail     per node: end - start(last descendant)  (or end - start, per tape)     2|4 B
codes    per code: kind and field, flags, alias, up to 8 supertypes            6+2k B
aliases  per alias: name and named flag
```

Each finding is tagged:

- **[F]**: inherent to the format. A new format can remove it.
- **[I]**: it comes from the current code. It can be fixed without changing the format, but it
  shows how the format is actually used.

Paths are relative to this repository (the hub, `e3776af9c`) unless a worktree is named.
"PG" is the Parser-generators worktree, "NGS" is Native-and-GPU-strata.

## 1. The layout makes every writer hold the whole file

1. **[F] Nothing after the code column has its final place until the file ends.** The size
   column starts at `padded(64 + 2n)`, so its place depends on the node count, and so does every
   later column's. Each writer therefore keeps size, off, tail and base in buffers of their own and
   copies them into place at `finish` (`crates/tape_format/src/lib.rs`, `Writer::finish`;
   decision 0140, "the user's choice").
2. **[F] The header is written last.** It holds the node count, the code count and the code table's
   length, so no prefix of the image can be streamed out.
3. **[F] Size and tail are known only at a node's close.** Every writer goes back and patches cells
   it wrote earlier (`Writer::close`).
4. **[F] Wraps and edits rewrite the tail of the image.** A node found after its first child (an
   operator after its left operand) is inserted before it. That is a memmove of the code, size and
   tail columns from that point on, the base and offsets recomputed for every block from there to
   the end, and the codes renumbered (`Writer::insert`). A chain of such wraps is quadratic in the
   chain's length.
5. **[F] Streaming within the memory budget was required and never shown.** Decision 0025 says "the
   production encoder must stream within the 500 MB budget", next to a harness that took 783 MB for
   31.5 MB of source. Points 1–4 make streaming impossible, and nothing records a check.

## 2. Codes numbered per tape, by first use

6. **[F] The same node gets different code ids in different tapes.** No grammar-wide catalog
   exists, so each tape numbers its own (decision 0025). The effects:
   - Every reader translates: `DeviceCode` per admission (`tape/resident.rs`), `kind_atom` per node
     in `Facts` (`facts/core.rs`), `by_code` groupings on the device.
   - SQL and cross-tape joins cannot compare code columns.
   - It is the store's largest repeat across files: `00 00 01 00 02 00…` starts 95–99% of tapes.
     A per-grammar numbering is listed but was never measured (t28).
7. **[F] Numbering is serial and depends on order.** A GPU or any parallel writer needs a global
   prefix order. Doc 13 has the device guess the numbering, check it with `atomic_min` and
   renumber.
8. **[F] Edits before the commit point renumber the suffix of the code table.** In the lean
   writer, renumbering at every edit cost 1,326M instructions on 1,360 `.rr` files. Commit points
   brought that to 620.8M (doc 35). Code ids are still the writer's largest cost: 32 of the 49
   instructions a node it spends above the floor (keys and starts only).
9. **[F] Writers keep a 1 MiB key→id table per thread.** It is reset as each tape's codes are
   dropped (`Writer`'s `IDS`). The tree-sitter path does the same with an `FxHashMap` over the
   24-byte `Code`, which was 29% of pack in debug.
10. **[F] Supertype memberships are folded into the codes, from the parser's cursor.**
    - A membership depends on where a node sits, not on its kind alone. A lookup by kind matches only
      74–81% of nodes; one by (parent kind, field, kind) misses 8–152 nodes a corpus, and no grammar
      file gives that table (decision 0025).
    - The tree-sitter encoder makes a raw FFI call (`ts_tree_cursor_current_status`) for every node,
      guarded by layout assertions (`tape.rs`, `cursor_supertypes`).
    - Producers with no cursor cannot make memberships at all: the lean and tier 2 writers write
      `class_count 0`, so tier 2 cannot yet tape a grammar that has supertypes.
11. **[F] One kind spans many codes.** Field and context are folded in: 168–227 codes a grammar,
    502–694 once named-only (t27). Label passes make a segment per (document, code), not per kind
    (`metal_label_passes.rs`).
12. **[F] Fixed limits are checked by every producer.** These are kind 10 bits, field 6 bits, u16
    ids, at most 8 supertypes and u32 offsets.
    - A node with a 9th membership fails the whole file (`tape.rs`, "too many supertype
      memberships").
    - A grammar with more than 1,024 kinds cannot be taped at all.
    - The checks are repeated in `encode_node`, `Code::plain` and `json_parser/tape.rs`.
13. **[F] `extra` is per node, but one Python kind varies** (1 node in 3.2M). It needs a minority
    code. PG's tier 2 decides `extra` per kind at `finish`, so it cannot represent that kind; it
    also has no ERROR or MISSING flags.

## 3. Widths chosen per document, known only at the end

14. **[F] A column is 2 or 4 bytes per document, and must be the narrowest that fits.** A writer
    either guesses and parses again (`Redo`), or writes wide and narrows afterwards (simdjson off
    arm64). Writing 4 where 2 would hold also counts as wrong.
    - Re-parses: 7 of 436 JSON files (decision 0088), 7 of 2,200 with simdjson (0134), none of
      1,362 `.rr` files (0140). The GPU needs a second command buffer (doc 13).
    - `Widths::guess` writes sizes at 4 from 320,000 bytes, so nearly any larger `.rr` file with
      fewer than 65,536 nodes is parsed twice. simdjson does the same at 52,000 structurals.
    - PG's tier 2 compiles only narrow widths: a wide file goes to the fallback parser.
15. **[F] The rare wide documents hold a large share of the nodes.** Only 0–8% of documents need
    4-byte source widths, but in the data-layer sample those hold a third of all nodes (0025).
16. **[F] Every cell read branches on width**, on the CPU (`tape.rs`, `resident.rs`) and on Metal
    (`metal_tape.metal`). Moving to v9 made sequential scans slower: 2.0 → 3.7 ns a node (0025).
17. **[F] Widths differ per document within one GPU batch.** This departs from the spike's design
    and is still "to be measured" (decision 0032).

## 4. Spans: base, off and tail

18. **[F] A node's end takes a chain of dependent loads:**
    `end = base[(i+size-1)/16] + off[i+size-1] + tail[i]`. That is a random access to the last
    descendant, plus the `TAIL_FROM_START` branch and the width branches. The GPU pays it on every
    node it reads (`metal_tape.metal`).
19. **[F] One early-ending node switches the whole tape to `TAIL_FROM_START`.** The writer re-parses;
    every reader takes the other branch; and since a tail measured from the other place says nothing
    of its width, a file can take three passes (simdjson allows `attempt < 3`).
20. **[F] `base` is the least start in each block of 16.** Wraps and zero-width nodes can start
    before the block's current base, so the block's base and its 16 offsets are recomputed.
21. **[I] Once any wide or early mark exists, the writer goes slow for good.** `wide_from` becomes 0,
    so every later push recomputes its block, and `mark()` searches the marks linearly
    (`lib.rs:309`, `:426`, `:445`). A tails-from-start file marks every early node: O(16·marks)
    a push on its second pass.
22. **[F] Line and column are not on the tape.** They are computed from the source on each read.
    This once made the `.rr` reader quadratic: 7.1 s, down to 54 ms with a line index (0074). A
    document can now have up to four line tables: `LineTable`, lazy `Points`, Metal's own `\n`
    scan, and the resident line starts. `resolve` builds a fifth.

## 5. Structure: no parent, and preorder as identity

23. **[F] There is no parent column.**
    - Every reader rebuilds parents with a stack: `restore_tree`, `prefilter/tapes.rs`,
      `resident::parents`, `tape_format::decode`. That is 4.1–5.1 ns a node each time.
    - In SQL, interval containment instead of a parent climb makes `inside` about 2× slower.
    - The spike's rec6 stored a parent delta for readers that read in place. Decision 0026 drops it
      because nothing reads in place, which item 29 says is itself a problem.
24. **[F] There are no sibling links.** Previous sibling, k-th child, child count and
    `child_by_field_id` are scans (`tape.rs`, `tape/navigation.rs` counts scans over 64). The GPU
    builds child lists (8 B a node) at admission, and `NodeTable::operator[]` rebuilds parent,
    next, next-named, span and code on every read, whatever the caller needs
    (`metal_batch_runtime.metal`).
25. **[F] Preorder index is the node's identity, and every position is absolute.**
    - An insert shifts every later preorder index, every later block's base and every ancestor's
      size.
    - A 9-line commit changed 929k input rows (open items). Per-definition ids and granule order
      keys were built on top to recover (0121, `facts/vtab/syntax/granules.rs`).
    - `TapeDoc::do_edit` is refused, `@splice` re-parses the whole text (0058), and a one-line edit
      re-parses, re-tapes, re-validates and re-projects the whole file.
26. **[F] A tape is one whole file, and an embedded program is a whole separate tape.** Each guest
    has its own header, code table, copy of its text and source map, and is parsed and encoded
    separately. A document derived from a derived document cannot be restored, so its file is parsed
    again (`tape.rs`, `restore_with_lines`).

## 6. Tokens the grammar implies are not stored

27. **[F] Leaving out implied tokens makes writing depend on the siblings.**
    - Whether a child is kept depends on an automaton run over all of its parent's children. The
      tree-sitter encoder walks each parent's children twice and builds a `Vec<bool>` mask per
      parent before it descends (`tape/tokens.rs`).
    - Readers that need a gap ask the automaton again per parent, keyed by strings, with a
      `HashMap` of fields and owned kind `String`s per call (`stored_gaps`). `TokenGrammar::of`
      takes a process-wide mutex per call, including once per encode.
28. **[F] It changed what rules mean, and some of that is still open.**
    - It reverses a finding of the spike, whose named-only trial left 210 shapes it could not
      translate.
    - S7 lost 140 keys and gained 3 false positives before the readers were rewritten (t27).
    - A `+` run no longer stops at an implied token: tree-sitter finds 3 matches, the tape 1 (open
      item O-repetition-implied-boundary).
    - Files of one shape now share tape bytes (`exists("file")` and `isfile("file")`), so a tape's
      identity has to include the source hash (t31).
    - Classification depends on `grammar.json`, which is part of the build identity. Rust's
      automaton was never finished, so its tapes keep every token.
    - The named-only regression (88.4 s against 22.9 s parse CPU, 857 MB against 316 MB peak) was
      never explained or measured again after format 14 (t30).

## 7. At rest: whole zstd frames

29. **[F] Nothing reads the stored columns at random.** Tapes are zstd frames decoded whole before
    any read (0026). Decision 0025 chose v9np because "random access is needed for proper anchor
    indexing", and then "nothing reads the stored columns randomly yet". The 16-byte column
    alignment pays off only in the GPU slab.
30. **[F] So pure SQL cannot read a tape.** The byte decoding in `tape/v9np/sql.rs` works only on
    raw blobs. In practice SQL reaches tapes through `nera_size` / `nera_tree`, which zstd-decode,
    copy and rebuild parents per tape behind an 8-entry linear LRU and a shared mutex, taken once
    per child hop (`prefilter/tapes.rs`, `prefilter/tree.rs`).
31. **[F] Small tapes are mostly fixed overhead.** The header and code table are a third or more of
    a small frame, and 152k of 179k tapes are under 512 B (t28). Named-only tapes need a zstd
    dictionary of their own; the full-tape one gains only 0.97×.
32. **[I] Small frames are compressed twice**, plain and with the dictionary, keeping the smaller
    (`content_store/compression.rs`, `compress_as`).
33. **[F] A tape's identity is a hash of its exact bytes** (with the source hash). Any writer
    difference changes residency, memo and derivation keys for the same tree. So every writer must
    match the others byte for byte, which is enforced by oracle tests. And any format bump cascades
    into the prefilter, positional index, core, facts and memo stores (t30; format 14's commit).
34. **[I] One build id covers every grammar.** It also hashes `Cargo.lock`, Blade, `rr_syntax` and
    `tokens.rs` (`native/build.rs`). A comment changed in `tokens.rs` re-derives every language's
    tapes, then the search index and memos downstream.

## 8. Readers turn the tape into something else, every time

The tape is not what the engine walks. Each consumer builds its own per-node structure, and
redoes it per phase.

35. **[I] `TapeDoc` builds a second tree of 40-byte `Rec`s.** Each holds kind, field, start, end,
    size, parent, first, next, prev, children and flags, about 4–5× the tape, beside a clone of
    the whole image (`restore_tree`). Holding trees costs 23.7× the source (0051). Research
    planned this rebuild to be lazy; it is eager.
36. **[I] There are at least eight more per-node forms:**
    - `Facts` columns, about 39 B a node with a child CSR and kind postings (`facts/core.rs`).
    - `Projected`, which renumbers again, with a granule numbering hashed per projection
      (`facts/vtab/syntax.rs`).
    - `MetalNode`, 44 B a node plus 4 B a child, still built on the non-resident path though the
      spec deletes it (`packed_batch.rs`).
    - Resident admission's parents, child starts, kids, `by_code` and line starts, about 16 B a node
      and 24 B a code (`residency/slabs.rs`).
    - `FileAnchorIndex`, about 100 B a node, rebuilt per evaluation, which does not use the stored
      search index's postings (`matcher/anchors.rs`).
    - The search index's kind stream, which stores the code column again as one `k<base36>`
      token per node, made with a `format!` per node (`search_index.rs`).
    - `.rr` statements, in five forms: frame → `TapeImage` → `Facts` → `Vec<rr::Node>` → `Tree`,
      with a `Vec<Vec<u32>>` child list (`logic/rr/from_tape.rs`).
    - Typing-support writes `node`, parent, `child` and `field` out as `.facts` text and parses them
      back into a CSR (Typing-support, `typing/rounds.rs`).
37. **[I] SQL projections hash per node.** `tree_digest` and `def_digest` compute one or two
    SHA-256s a node, as 64-character hex strings in the string table: 1.5 s of 2.8 s on 237 files
    (`core_rr.rs`).

## 9. Repeated work on every read

38. **[I] The source is hashed again on every restore.** `validated::visit` hashes the whole source,
    on top of the store's check and the workspace's, which makes two or three hashes per read. The
    spec says restores "must not re-hash source" (`rule-engine-direction/spec.md:474`).
    `encode_rr` and `pack` hash it again after parsing (14.2M instructions for the `.rr` corpus,
    0140).
39. **[I] Every restore validates in full**, not once at admission: every code against the grammar,
    every span against UTF-8 boundaries, sibling order and parent containment (`validated.rs`).
    With `keep` set, `facts::core::from_image` and `kept_tape` validate and hash twice.
40. **[I] The bytes are copied about four times from store to reader.**
    - The decompressed cache hands out `to_vec()` copies (`objects.rs`).
    - `read_image` copies each column out.
    - `restore_tree` clones the image.
    - The source is cloned.
    - Tape identity calls `to_bytes()` to hash, and `admit_docs` serializes and hashes twice a
      document.
41. **[I] Each phase repeats the chain:** restore (0.7 ms a document), facts (2 ms), projection.
    That was 1,788 document reads for 596 files (Matrix-offload-spikes `f01bc8785`).
42. **[I] `Grammar::new` rebuilds its kind-name table (up to 1,024 entries) on every call.** It is
    called per file. `kind()` and `is_named()` binary-search `kind_overrides` on every call.

## 10. Writers: many paths, intermediates, re-runs

43. **[I] Only simdjson and the lean `.rr` parser write the image directly.**
    - Tree-sitter, Blade and Bitlex JSON build `Pending` (about 56 B a node, an aliased node
      cloning its name into a `String`), then `finish`/`pack`. Pack makes about eight passes:
      sizes from the back, parent check, supertype sort, code fold, bases, maxima, columns, SHA-256.
      It costs 4,530 instructions a node in debug (543 optimized), against 1.1 G for the parse
      itself (open items, O-tape-pack-cost).
    - Bitlex JSON throws away the sizes its rows carry and adds them up again.
44. **[I] Tier 2 in the hub still writes 24-byte `NodeRow`s,** and its fallback adds their sizes from
    the back (`native_cpu_stream.rs`, `rr_parser/src/lib.rs`). In PG (uncommitted), tier 2 writes
    the image, at +32% instructions (Cranelift) and +22% (AOT) over the node table. It keeps a
    12-byte working row per node and copies its columns into the image at `ACCEPT`.
45. **[I] Even the direct writers copy after writing.** The `.rr` image is moved into place by
    `finish`, copied out column by column by `read_image`, and concatenated again by `to_bytes` for
    storage. simdjson writes separate `Vec`s in another order and goes through `written`, then
    `to_bytes`.
46. **[I] The block, commit and finish logic is implemented four times:** `tape_format::Writer`,
    the Cranelift IR generator and an AOT Rust template held in a string (both PG), and the simdjson
    C++ walker. Constants are copied too: `CHECKPOINT_STRIDE` from the old rec6 wire codec,
    simdjson's own `STRIDE` in Rust and C++ (it does not depend on the format crate), and whole local
    definitions in NGS, the named-only tapes branch and Transducer-demos.
47. **[I] PG's tier 2 writer hard-codes format 11's header offsets** (`56 + 8*k`, `image[24..56]`
    for the digest, `u64_at(56)`). They will be wrong on merge into format 15. A format kept only
    in a backend was deleted once already (open items, O-bitlex-writes-tape).
48. **[I] Room is guessed, with a re-run when it runs out.**
    - Tier 2 and the lean parser size for 1.5× the tokens, and actual nodes are about 0.76 a token.
      The columns are zeroed up front, which decision 0140 lists as remaining cost.
    - simdjson reserves `len + len/4 + 2` nodes, then `shrink_to_fit` copies.
    - Tier 2 runs the whole file again on overflow (`ROOM_RETRIES`).
49. **[I] A refusal costs a full second parse.** A file simdjson refuses goes to tree-sitter, a file
    tier 2 refuses goes to the reference parser, and `TapeImage::parse` tries the tree parser, then
    tree-sitter.
50. **[I] There is no GPU tape writer.** Metal tier 2 runs the CPU JIT over the device's buffers.
    The format itself blocks one: serial numbering (7), per-block minima (20) and sizes known at
    close (3).

## 11. Content the format still gets wrong

51. **[I] Blade's compiled view still drops nodes over placeholders** (`omitted` in `tape.rs`). That
    is against the requirement that Blade stores exactly what tree-sitter-php does (G4). Open.
52. **[F] The image carries a per-tape alias string table.** The requirements keep per-file alias
    tables off the tape. Decision 0025 measured no aliases, so this path has never been exercised.
53. **[I] JSON's tape tag (251) is a test-only tag** that was never decided. JSONC files and nesting
    past 1,024 fall back to tree-sitter.
54. **[I] A legacy 3-byte rec6 checkpoint limit (16 MiB) is still in `tape.rs`.**

## What a new format must keep

These are stated requirements, not preferences. Each is quoted from where it was decided.

- **Completely regular.** v4–v7 failed on overflow pairs, sorted flag lists and alias tables
  (`00-requirements.md` G2).
- **Processable in SQL with the SQL index prototype techniques**, including the pure-SQL fallback over a
  tape table function (G7).
- **No size increase.** 8–11 B a node was rejected, and a change must show clear benefits (G3,
  0025). The user, 2026-11-13: for format 16 this is a tradeoff to measure
  (bytes against GPU decode and random access, 0156 T1), not a bar set beforehand.
- **Random access**, for anchor indexing (0025).
- **The only per-node structure stored.** Everything else is a bare index or an aggregate of at most
  0.25 rows a node; no sidecars and nothing keyed by the digest (0026, G4; enforced by
  `no_relation_stores_a_row_per_node`).
- **One source, one grammar, one tree per tape**, and no splicing (G6, G9).
- **Blade as tree-sitter-php stores it, and ERROR and MISSING spans as tree-sitter emits them**
  (G4, G5).
- **At-rest and resident forms designed together**: columns readable in place on the device (F5,
  0032).
- **Memory within 1 GB** (was 500 MB), with pinned GPU buffers counted, and an encoder that streams.
- **Named-only, with no implied tokens**, decided from the grammar alone. Nothing rebuilds a
  left-out token (formats 14 and 15, t31).
- **One definition, and byte-identical output from every writer** (0140).
- **Parsers write the tape directly, with no pass after them** (doc 12, 0140).
- **GPU: masks and tape in one dispatch in the common case** (doc 13).
- **Keyed by content, grammar version and format version**; identity is (tape bytes, source hash)
  (0051, t31).
- **Kind codes local in the tape, global in indexes.** Global codes in the record were rejected at
  +16.7% (0055).

## Requirements the user added (2026-11-13)

The user listed five requirements after reading the survey above. Below, each is checked against
the code: where the current format stands, and what in the findings it touches.

**U1. A compression format the GPU can decode, which still compresses reasonably.**
The user's gpu-csv-parser (2019, `v8-decompress-on-gpu.cc`) shows the shape:
- an LZ4 frame of independent blocks;
- one thread walks the block headers to an index of (offset, length, output offset);
- then one threadgroup per block decodes, spreading each literal run and back-reference copy over
  its threads;
- line starts come from a map and a prefix scan (`v6`).

Today:
- Tapes and sources are zstd frames, decoded on the host before any read (`content_store/objects.rs`).
- Resident slabs are decompressed on the host into the slab (`residency/slabs.rs`, `plan`).
- Small frames use a trained dictionary.

For a design:
- 152k of 179k tapes are under 512 B (t28). Independent blocks that small compress poorly unless they
  share a dictionary, so the dictionary has to be resident on the device too. gpu-csv-parser's
  decoder refuses dictionaries.
- zstd's entropy stage (FSE/Huffman) is serial within a block; LZ4's literal and match copies are
  not. That is the reason to prefer the LZ4 shape.

Touches 29, 30, 31, 32.

**U2. The same structure repeated gives the same bits, without hurting decoding.**
Today a repeated subtree gets different bytes:
- `base` and `off` are absolute positions.
- Code ids depend on what came first in the file.
- The widths depend on the whole document.
- The code table is per tape: `00 00 01 00 02 00…` starts 95–99% of tapes (t28).

Size and tail are already relative. Making positions relative too, with absolutes recovered by a
prefix scan (gpu-csv-parser's `v6` does this for line numbers), would make repeats byte-equal, which
the compressor can then find. It also bears on edits (25) and on identity by bytes (33).

Touches 6, 7, 14, 18, 20, 25, 33.

**U3. Room for a second hash when SHA-256 is deprecated, without wasting bits.**
Not in the survey above.
- The store is half ready. `hashes` has an `algorithm_id` (SHA-256, git SHA-1, git SHA-256 and
  BLAKE3 are defined), but each hash is a fixed four 64-bit lanes (256 bits:
  `content_store/schema.rs`), so a longer digest does not fit.
- The engine is hard-wired to SHA-256:
  - `tape_identity` returns `[u8; 32]` and expects a SHA-256 identity;
  - `validated::visit` and `encode_rr` hash with `Sha256`;
  - 83 files under `code_rule_dsl` and `content_store` hold `[u8; 32]`.
- The tape itself carries no digest since format 12 (it was 32 B, about 7% of compressed tapes).
  So the bits a second hash needs belong in the store's rows, keyed by algorithm, and the format's
  job is only not to bake one algorithm into identity.

**U4. Loading a tape without the full source, with a compact side form that rebuilds the text that
is not implicit.**
Not in the survey above as a goal. Related to 22, 27 and 39.
- Today every restore needs the whole source: `validated::visit` hashes it and checks UTF-8
  boundaries, `TapeDoc` holds it, `MetalNode` copies it, and readers rescan it for tokens and lines.
- Compressed tapes already cost more than the text they describe: named-only tapes are 1.3–1.8× the
  zstd'd source on every language in t28's table (Python 2.81 MB of tape for 1.74 MB of source,
  Rust 3.05 for 1.72). So structure plus a side form of the non-implicit text (identifiers,
  literals, comments, layout) could cost less than tape plus source.
- The grammar already decides which tokens are implicit (formats 14 and 15); that machinery is
  what tells the side form what it may leave out.

**U5. Files over 4 GB (up to 1 TB, at most 1 PB).**
Item 12 noted u32 offsets only in passing. Today:
- `encode_tree_image` and `encode_rr` refuse sources over `u32::MAX`; `validated` and `resident`
  refuse more than `u32::MAX` nodes.
- Starts, ends, bases and every per-node record (`Rec`, `Facts`, `MetalNode`) are u32.
- The header's source length is already u64.
- SQLite holds a blob of at most 2^31−1 bytes (1 GB by default). So a file that large cannot be
  one stored frame whatever the format, and has to be stored in blocks, which U1 wants anyway.

Bounds: 1 TB is 2^40 bytes and 1 PB is 2^50. Positions relative to a block (U2) keep the per-node
fields narrow; only block-level bases need 40–50 bits.

## Where the costs sit

Most per-read cost is in the implementation (sections 8 and 9: hashes, validation, copies and
conversions, each redone per phase). A new format alone does not remove it. It can remove the
reasons that work exists:

- The tape is not what readers walk. With no parent, no sibling links, ends behind a chain of
  loads, and per-tape code ids, every reader builds its own tree.
- Nothing is final until the file ends: the column places, the widths, the tail origin and the code
  numbering. That rules out streaming, parallel writing and GPU writing, and it causes re-parses.
- Preorder as identity, with absolute positions, makes every edit a whole-file rewrite, and keys
  everything downstream to exact bytes.
- Whole-frame zstd makes the random access the format was chosen for unreachable at rest.

Some figures here are quoted from the decisions and notes cited, not re-measured for this survey.
In particular: the 4,530 instructions a node for pack (debug), the lean writer's instruction
counts, the tier 2 percentages and the named-only regression.
