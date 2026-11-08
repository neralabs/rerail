# Tape regularity: v4 → v7, the spike's format experiments, multiple trees per file, and a regular v8 candidate

This report is read-only. Its sources:

- **Rerail tape history.** `native/src/code_rule_dsl/tape.rs` at each version commit. Copies are in `scratchpad/tape/tape-v{3,4,5,6a,6b,7}.rs`.
- **Rerail tape modules.** `tape/wire.rs` and `tape/index.rs`, and DLRE's SQL anchor store (`matcher/anchors/sql/`).
- **Executor and Tempo tapes.** N1562 `packed/metal_batch_abi.rs`, and Tempo `nera-landing-2/ner-1562-scale-test-the-re-railer/tempo/native/src/policy_dsl/tape.rs`.
- **ast-index-spike** at `9449260`: `DESIGN.md` (D), `DESIGN_HISTORY.md` (DH), `README.md` (R), `results/storage_report.md` (SR), and the two indexes in `docs/rule-engine-direction/prior-work/`.

## 0. Headline

1. **The record encoding has not changed from v3 to v7.**
   - Every Rerail version is the spike's `rec6` v2 tape: a 6-byte `[kf:2][par:2][size:2]` record with `0xFFFF` sentinels spilling to sorted overflow tables, spans as two varints per node (`zigzag(Δstart)`, `len`), and a 6-byte checkpoint every 16 nodes.
   - `tape/index.rs` says so itself: "The spike's rec6, span checkpoint, and overflow columns".
   - **What made v4–v6 "highly irregular" was multiple grammars and multiple trees inside one tape:**
     - v4: a Blade tape held the PHP it embeds, with kind codes shifted by an offset;
     - v5: embedded programs were grafted under the host root, with renumbered combined codes and spans projected into host coordinates;
     - v6: guest tapes were nested inside the host image, each with its own text and segment map.
   - **v7 made the *document* regular:** one tape holds one source, one grammar and one tree.
   - **v7 did not make the *record* regular.** Several irregular features remain:
     - spans are varints;
     - parent and size use sentinel escapes;
     - flags live in sorted side lists;
     - ERROR is a code 0 plus a list lookup;
     - contextual kind aliases are a per-file string table plus an override list.
2. **Version numbers collide.** Tempo's `TMPTAPE4` (nera-landing lineage) and Rerail's `TMPTAPE4` share a magic but have different tables. Tempo's has `supertypes`; Rerail's has `syntax_errors`. Tempo's Metal executor is still on Tempo's v4.
3. **Regularity is lost at exactly the fields a SQL, GPU or JIT consumer needs per node:** span start/end, the error kind, the extra/missing flags, aliases, and parent and size in large files.
   - The consequence is visible in DLRE. Its SQL path rebuilds row-per-node caches (`decoded_properties`, `decoded_spans`, `decoded_children`), which is the spike's rejected sidecar shape (D S089).
   - N1562 expands every node to a 44-byte `MetalNode` before any GPU or JIT work.
4. **v7's multi-document model is the spike's S070 model**, made more general: derived text can be generated, so the map is a set of disjoint ranges.
   - **v5 re-learned a spike dead end.** Splicing foreign trees into one tape needs a global kind space and cross-language parent/size arithmetic. The spike rejected it (D S070 L4150-4158), and Rerail went v5 → v7 to undo it.
   - **One spike lesson is not adopted on main:** global anchor codes. Main's `search_kind_fts` tokens are `k%04x` of the *local* code, so kind postings and `fts5vocab` frequencies mix languages. DLRE's anchor store does use `LANG_STRIDE = 1024`.
5. **A completely regular v8 is cheap.** Two facts make it so:
   - Parent and size can overflow 16 bits only in files with more than 65,536 nodes, so a **per-document column width** (u16 or u32, declared in the document header) removes every escape table at no cost for normal files.
   - Spans at fixed width (u16+u16 when the source is under 64 KiB, else u32+u32) replace varints and checkpoints.
   - The logical size goes from the spike's 8.66 B/node to about 11 B/node for normal documents (about 19 for very large ones). Compression is applied only at rest.
   - The spike's own trend supports paying this: random-access size, and each step of the checkpoint stride down to 16, both paid for themselves.

---

## 1. Version-by-version table

Legend: **IRR** marks an irregularity, meaning node *i* cannot be decoded with fixed-stride arithmetic alone.

| | ast-index-spike rec6 v2 (D S047) | Tempo/nera-landing "v4" | Rerail v3 (`042d82eb`, 2026-10-08, port from nera-landing) | Rerail v4 (`aac1ff23`, 10-13, Forte Blade) | Rerail v5 (`2a303bfa`, 10-15, multi-region embedding) | Rerail v6 (`08053152`, 10-16, documents first pass) | Rerail v7 (`4e57c59d`/`1fbb06c7`, 10-16/19, "documents as their own tapes") |
|---|---|---|---|---|---|---|---|
| Header | none; per-file columns in `t_file` | magic, lang, version, len, digest, table lengths | magic `TMPTAPE3`, lang, 3 reserved, version, source len, SHA-256, 11 u64 lengths | `TMPTAPE4`; 12 lengths (adds syntax_errors) | `TMPTAPE5`; adds program count | `TMPTAPE6`; `HEADER_BYTES = 56+12·8` | `TMPTAPE7`; `HEADER_BYTES = 56+11·8` |
| Record | 6 B `[kf:2][par:2][size:2]`, `kf = kind10 \| field6<<10` | same | same | same | same | same | same |
| Parent | delta `i - parent` (root `i+1`); `0xFFFF` → `parovf` (**IRR**) | same | same | same | same | same | same |
| Size | subtree node count; `0xFFFF` → `sizeovf` (**IRR**) | same | same | same | same | same | same |
| Spans | varints `zigzag(Δstart)`, `len` (**IRR**: sequential) | same | same | same | same | same | same |
| Checkpoints | K = 1024 → 128 → 16; `[spans_off:3][start:3]` (**IRR**: needed for random access; 3 B caps at 16 MiB) | K=16 | K=16 | K=16 | K=16 | K=16 | K=16 (`CHECKPOINT_LIMIT 0xFFFFFF`) |
| ERROR kind | — (PG/MariaDB/SQLite text paths) | code 0 + `error` list | code 0 + `error` list (**IRR**: kind needs binary search) | same | same | same | same |
| extra flag | in the kind dictionary `(name, named, extra)` (TAG L108-146); regular | sorted `extra` list (**IRR**) | sorted `extra` list (**IRR**) | same | same | same | same |
| missing | sparse list (TAG L147-157) | `missing` list (**IRR**) | same | same | same | same | same |
| Contextual aliases | interned at registration; `code_ctx` spare codes (DH L1858-1909) | `kind_aliases` (string table) + `kind_overrides (pre, alias)` (**IRR**: per-file strings) | same | same | same | same | same |
| Supertypes | — | `supertypes (pre, u16)` per node (**IRR**) | — | — | — | — | — |
| Syntax errors without records | — | — | — | `syntax_errors (start, end)` byte ranges (Blade) | same | removed (Forte errors became ERROR leaves) | — |
| Multiple grammars in one tape | **No.** Segments are separate tapes (S070); global codes only in the anchor index (S071) | — | — | **Yes**: Blade tape + compiled PHP view, PHP codes shifted by an offset; spans via `View::span` (**IRR**: mixed code space) | **Yes**: `programs[(root, dialect, apart)]`; programs grafted under node 0; codes renumbered through `Embedded` numbering; spans projected to host coordinates (**IRR**) | **No more grafting**: `guests: Vec<GuestImage{via, text, segments, host_errors, tape}>` nested in the host image (**IRR**: variable nesting in one blob) | **No**: one source, one grammar, one tree per tape. `FileImage{primary, derived[]}`; each `DerivedImage{role, via, parent, text, source_map, host_errors, tape}` is its own source and tape row, linked by the store's `file_documents` table |
| Rationale (commit text) | budget-driven re-pack (D S039) | — | port | Forte replaces tree-sitter-blade | "multi-region embedding" | embedded programs as documents | "every tape holds one source, one grammar and one tree", disjoint source maps, half-open ownership (4e57c59d message) |

**Constant from v3 to v7:** `KIND_LIMIT 1024`, `FIELD_LIMIT 64`, `WIDTH 6`, `CHECKPOINT_STRIDE 16`, and the u32 counts. The restored in-memory form (`TapeDoc` `Rec`) is fully regular: kind, field, start, end, size, parent, first, next, prev, children (u32) and four bools, about 40 B/node. Regularity is therefore lost only on the wire and at rest, and every consumer pays an O(n) restore to get it back.

**Three representations exist today:**

1. **The content-store `TMPTAPE7` image.** zstd over the whole blob, so it is not randomly accessible.
2. **DLRE's anchor-corpus copy** (`matcher/anchors/sql/schema.sql`, `file` table):
   - spike-style columns `rec`, `spans`, `checkpoints`, `parb`, `sizb`;
   - a **regular 1-byte `flags` column** (named | extra | missing | error | first_field), a real improvement over the side lists;
   - `parovf`/`sizeovf` as WITHOUT ROWID tables;
   - a 128-byte `kmask` (spike S227);
   - demand-filled `decoded_*` row tables.
3. **Executor ABIs.** `TapeDoc.Rec` in memory, and N1562's `MetalNode` (44 B AoS u32: parent, next_any, next_named, child_start, child_count, descendant_end, source_start, source_end, kind, field, flags) plus a child array and `MetalFile{node_start, source_start, line_start, …}`.

## 2. Why regularity matters, and exactly what in v7 breaks it

**Consumers:**

- **SQL** (SQLite blob I/O or `substr`). Node *i*'s field must be `substr(col, i·w+1, w)`, with no loop and no join.
  - Varint spans force a walk from the checkpoint of up to K−1 nodes. This was the spike's dominant per-query cost, which S251 measured at 26 spans decoded per span wanted at K=128.
  - Overflow forces a correlated lookup per row: about 509K buffer hits, fixed only by a CASE guard (DH L1461-1482).
  - DLRE paid for the irregularity by materializing `decoded_properties`/`decoded_spans`/`decoded_children` rows, which re-creates the S089 sidecar (spike: 211 B/node on SQLite, 88% of the store).
- **GPU (Metal) and CPU JIT transducers.** These need coalesced fixed-stride loads and no data-dependent branch per node.
  - Sentinel checks plus binary-searched overflow, varint decode and sorted-list membership tests are all divergent branches and dependent loads.
  - That is why N1562 expands to 44 B/node `MetalNode` before launch, and why Tempo's full-corpus run spends memory on expansion (about 2 GB tracked).
- **Random access by preorder index** (anchors from postings, demand points, rehydration of one finding). The ideal is O(1) per field; today span start/end is O(K) and error/extra/missing/alias are O(log n).

**v7 features that break fixed-stride O(1) access to node i:**

| Field of node i | v7 | Breaks because |
|---|---|---|
| kind | `kf & 1023`, **except** ERROR (code 0 + `error` binary search) and aliases (`kind_overrides` search → string table) | Lookups for ERROR and aliases; the alias name is a string, not a code |
| field | `kf >> 10` | regular |
| parent | `i − par`, **except** `0xFFFF` → `parovf` search | escape |
| subtree size / end | `size`, **except** `0xFFFF` → `sizeovf` | escape |
| span start | checkpoint ⌊i/16⌋, then up to 15 varint pairs | sequential |
| span end | as start, plus `len` | sequential |
| named | grammar table by kind, **but** alias named-ness comes from the alias string table | lookup |
| extra / missing / error | sorted lists, binary search | lookup |
| first_field / child ordinal / prev sibling / next named | not stored; restore computes them | needs an O(n) pass, or a climb |

## 3. Multiple trees per file

**v7:**

- A file's documents are the primary plus derived ones. Embedded is Vue `<script>`; generated is Blade's compiled view.
- Each document has its **own text** (Blade's compiled PHP is not a substring of the file) and its own single-grammar tape.
- A `SegmentMap` maps the document's text to *disjoint* parent ranges. Whitespace between ranges is joined, and the convex hull is used only for pruning.
- `host_errors` are parent bytes the document cannot be read through.
- Each document is a content-addressed source and tape row, related through `file_documents(role, via, parent, source map, host errors)`.
- Zero-width positions are owned half-open.
- Embedders run on the primary only; nesting is representable but unused.

**ast-index-spike** (D S070 L4148-4190, S071 L4190-4240; DH L1754-1837):

- **Model.** A multi-language file is "several single-language files joined together".
  - Each embedded region is parsed standalone and stored as its own tape row (its own `fid`).
  - A sparse map `t_seg(seg_fid, parent_fid, lang, byte_start, byte_end, tape_begin, node_count)` has no rows for single-language files.
  - A segment's offsets are relative to its region, and a match maps back by a byte shift. Segments are named `host#byte` (DH L3342).
  - `language_at` and `language_ranges` come from `t_seg` without decoding the tape. The innermost segment wins.
- **Rejected:** splicing foreign nodes into one tape. It "forces a global kind space the 10-bit kf cannot hold, and cross-language par/size math".
- **Cost:** +11% storage (400 HTML); `t_fk.lang` +0.042 MB.
- **Multi-language tax:** 13% (Python in Markdown, 132 vs 117 ms), from local codes colliding in the anchor index.
- **Option A,** global codes in `rec`, would have widened the record from 6 to 7 B (+16.7%) for every file. Rejected; it is D9.
- **Option B,** global codes `lang_id·1024 + local` in the anchor index only, removed the over-scan exactly:
  - FTS instances: 320,767 → 180,005;
  - SQLite 565 → 536 ms, PostgreSQL 263 → 224 ms, MariaDB 428 → 332 ms;
  - parity held.
  - The record keeps local codes, which is safe because the global-code candidate filter has already confined candidates to one language.
- **Limitation:** segments were contiguous substrings parsed standalone. The spike never handled generated text or disjoint multi-range regions.

**Assessment:**

- v7 = S070, generalized correctly for generated and multi-range text. v4–v6 were the rejected splice design and its partial unwinding.
- **Gap:** main's content-store kind index has no Option B. Its tokens are `k%04x` of the local code, so every language's postings share tokens. Two consequences:
  - Anchors over-scan.
  - **The `fts5vocab` kind statistics this direction relies on (README "Statistics") mix languages.** Fix: tokens `k{lang·1024+local}`, which DLRE's store already does, or scope by grammar.

**A regular representation for multiple trees:**

- **At rest:** keep one regular tape per document. The document is the content-addressed, incremental and deduplicated unit, as v7 and the spike agree.
- **Per file:** a fixed-width document table, with each row
  `doc, parent_doc, role, via, lang, node_count, text_len, map_off, map_count, width_flags`.
- **Map:** a regular array of fixed triples `(doc_start:u32, parent_start:u32, len:u32)`, sorted by `doc_start`. Mapping a document byte to a file byte is a binary search over the document's triples.
  - This is not on the hot path. Under `findings-contract.md`, location normalization happens once per finding, in consolidation.
- **`host_errors`:** a regular array of `(start:u32, end:u32)` pairs.
- **For execution batches:** concatenate the columns of all documents in the batch, with a batch document table `(node_base, text_base, line_base, lang)`, as `MetalFile` already has.
  - Node ids become batch-global by adding `node_base`; nothing inside a column changes.
  - Cross-document relations (`host`/`embedded` clauses) use the document table and the map, never parent/size arithmetic across documents.

## 4. ast-index-spike tape-format experiments

Each row gives the question, the variant, the numbers, the verdict and the citation, then what it means for Rerail and for v8.

| # | Question / variant | Numbers | Verdict | Cite | Bearing on Rerail (v4→v7) and v8 |
|---|---|---|---|---|---|
| E1 | mldb Lisp tape (atoms 8 B, list 16 B), in memory, O(1) skip by precomputed size | `()` 32 B → 16 B proposed | prior art; dropped when the database became the only representation | DH L232-262 | Rerail's in-memory `Rec` (~40 B) is in this family. v8 should make the wire form directly usable so restore is a copy, not a rebuild |
| E2 | v1 row per node (pre, rgt, parent, child, tok, 3 B-trees, per-leaf FTS) | 12.4x source, 102 B/node (Python); C++ 14.2x, 91 B/node | rejected (D1) | DH L265-290; SR L68-80 | DLRE's `decoded_*` tables and N1562's whole-catalog row materialization re-learn this. v8 must make them unnecessary |
| E3 | Lean rows (drop tok, op_id, size/length, no parent/child columns, contentless FTS5, zlib source) | 4.87x with kind B-tree, 3.43x without; partial rows 2.70x; plus identifiers in FTS only 1.80x; PostgreSQL rows 14.51x down to 6.93x | rejected | DH L296-377; SR L68-131 | same |
| E4 | v2 blob tape: packed varints + zlib per file + per-file FTS | 0.86x source, 7.1 B/node; structural payload ≈ 2 B/node | compact but not queryable in SQL (D4) | SR L77, L466-504 | Rerail's content-store image (zstd whole blob) is this. Fine at rest; not a query format |
| E5 | PostgreSQL per-file columnar arrays + `unnest` | 1.43x, 11.8 B/node; query 2.0 s | superseded (D3) | SR L109-155 | SoA idea later revisited (E11) |
| E6 | SQLite JSONB + `json_each` | 3.63x; naive CTE 8+ min; materialized 6.5 s + 298 ms | rejected (D2) | SR L157-192 | — |
| E7 | Binary tape + TVF vs pure-SQL nibble decode | zlib + TVF 1.08x, 2.2 s decode, 120 ms join; pure SQL needs uncompressed 2.46x, 94.7 s (43x slower); base64 1.35x | native decode is needed on SQLite; zlib is not expressible in SQL | SR L194-248; DH L390-403 | **v8:** the SQL-addressable form is uncompressed. Compression only at rest |
| E8 | PostgreSQL bytea + plpgsql `get_byte` | 1.49x; 27 min, killed | rejected | SR L250-302 | the "primitive matters" lesson (E12) |
| E9 | ASCII kind string, regex/LIKE over structure | 2.00 B/node raw, 0.18 zlib; adjacency exact in 50 ms | structure is context-free, so prefilter only (D7) | SR L304-340; DH L407-420; D S024-S031 | — |
| E10 | Wide relational node table (13 columns + 3 covering indexes) | ≈69 B/node, 13.9x; SQLite nested loop 48 s vs PostgreSQL hash 2.7 s | rejected | DH L462-489; D S014-S015 | same as E2 |
| E11 | Compact tape: TEXT kinds/fields streams + varint spans | TEXT `substr` O(n²), 314 µs/node | replaced by a fixed-width blob | DH L1017-1026 | **Regularity first appears here:** fixed width was 30x faster than variable-width text |
| E12 | `nodes` fixed blob 7 B `[kind:2][field:2][par_delta:3]` | 11 µs vs 314 µs/node (30x); decode 0.5 µs/node; but global cpos/spos window functions 37.6 µs/node (75x worse) | packed into rec6 | DH L1017-1031, L1142-1147; D S037 | **v8:** sibling ordinals must not be computed globally in SQL. Compute them in the executor, or make them an optional regular column |
| E13 | Random-access size: record 7 → 10 B (fixed size field) | storage 52.9 → 75.5 MB; tail queries 17.9 s → 3.38 s and 19.8 s → 3.40 s, complete (adjacency was 89% and 0%) | adopted, then re-packed (E14) | D S038 L2594-2651 | **Spike result v8 must follow:** paying fixed bytes for random access to a field the walk needs pays off (5-6x) |
| E14 | Re-pack to rec6 `[kf:2][par:2][size:2]` + `0xFFFF` overflow; 5 B variant (kind via FTS); succinct BP/LOUDS | 6 B: 45.3 MB + 0.37 MB overflow (par-ovf 0.31% = 23,331 rows, size-ovf 32 rows); 5 B: 38.1 MB, up-climb slower; succinct ≈2 bits/node, needs rank/select, not built | **6 B shipped** | D S039 L2652-2727; DH L1387-1401 | Rerail inherited exactly this. **v8 note:** overflow only happens when `node_count > 65,536` (delta < i < n; size ≤ n), so a per-document width replaces it with no escapes |
| E15 | Anonymous nodes stored or not | 7.55M vs 4.90M named = 1.54x; named-only loses 210 shapes, breaks `pre+1` first child, operator discrimination, anchors | **store all** (named-only reversed) | D S040 L2728-2754; R L1662-1712 | Rerail stores all (tree-sitter CST). **v8 keeps that** |
| E16 | Interleaved vs SoA under PostgreSQL TOAST | interleaved 7 B: EXTERNAL 7.37, pglz 0.70, lz4 0.75 B/node; SoA 7.47 / 0.73 / 1.03; random-access slices: compressed 9.5-17.3 s vs EXTERNAL 202 ms | "no storage reason to prefer SoA"; compression vs access dominates | D S041 L2755-2800 | Rerail at rest is zstd whole-blob (≈ compressed). **v8:** column layout is free to choose on storage grounds; compress at rest, uncompress for access |
| E17 | Forced-flat MATERIALIZED CTE (decompress once per file) | 17.3 s → 39.6 ms (nodes pglz); SoA pcol 22.8 ms, beating EXTERNAL 192 ms; needs about 63 positions per file to amortize | adopted on PG; SoA marginally preferred for flexibility | D S042 | Rerail equivalent: decompress a survivor's tape once per batch. **v8 should keep per-column access** so a batch reads only the columns it needs |
| E18 | Spans: varint pair with checkpoints, stride K | K=1024 → 128 (MariaDB 522 → 239 ms; +0.019 B/node; knee 32 for viewports); S251 K sweep on the linter set: K=128 6.34 s, 64 4.86, 32 3.89, **16 2.80**, 8 2.58; one `#match?` 100 → 45 ms; K=16 costs 0.376 B/node | K=16 adopted (twice revised); K was "never priced against its consumer" | D S044-S045 L2862-2962; DH L7739-7800 (S251) | Rerail fixed K=16. **The spike's trend runs toward K=1, i.e. fixed-width spans.** Each halving of K paid. v8 takes the limit and drops varints and checkpoints |
| E19 | Viewport (byte range) read | normal files < 1 ms; huge-file middle 120-330 ms with varint spans (O(position)); fixed by checkpoints | — | D S044 | fixed-width spans make this O(log n) by binary search on monotone `start` |
| E20 | Line checkpoints `lnck` `(byte_offset, line_no)` every 4 KB of source | 0.004 B/node | adopted; byte spacing beats line spacing; dense per-line vector rejected (D15: 33-67 MB) | DH L146-156, L201-204, L3591-3597 | Rerail rebuilds a `LineTable` from source per restore. **v8 option:** a regular line-start column per document (as `MetalFile.line_start`). Small, and moves line mapping off the source scan |
| E21 | Kind interning at grammar registration (not first observation) | stored bytes had depended on batch order | adopted (R17) | DH L18-26 | Rerail uses grammar catalog codes (registration-stable). **v8 should intern aliases the same way** (below) |
| E22 | Global anchor codes: Option A in `rec` vs Option B in the index only | A: +16.7%, rejected (D9); B: over-scan 320,767 → 180,005; −5 to −22% latency; parity | **B adopted** | DH L1789-1837; D S071 | **Rerail main does not have B** (`k%04x` local). DLRE does (`LANG_STRIDE`). v8: the record keeps local 10-bit codes; the index and statistics use `lang·1024 + local` |
| E23 | Contextual subkinds `code_ctx` (spare 10-bit codes per (parent, field, kind)) | +16 codes, record still 6 B; parity; tail not improved (T50); revived narrowly (S274) | narrow use | DH L1858-1909, L8531-8588 | **This is the mechanism for carrying aliases regularly:** spare codes in the same 10-bit space |
| E24 | Embedded segments as standalone tapes (`t_seg`) | +11% storage; 13% tax removed by E22 | adopted | DH L1754-1787; D S070 | = Rerail v7. v4/v5 splicing = the rejected alternative |
| E25 | Encoding metadata `srcenc`; source as bytes | non-UTF-8 file aborted queries; err 4 → 0 | adopted | DH L2935-2954, L7982-8012 | Rerail sources are UTF-8 strings; byte offsets OK. Keep a byte-addressed source |
| E26 | `extra` flag: kind dictionary `(name, named, extra)` | 0 matches without it; uniform per kind over 18.5M nodes; no per-node bytes | proposed for production | TAG L108-146 | Rerail stores a per-file sorted `extra` list. **The spike measured that extra is a per-kind property;** if that holds for Rerail grammars, v8 drops the list. Keep a per-node flag bit only where the kind is not uniform (verify on corpus) |
| E27 | `missing` sparse list; grammar fingerprint | needed if error trees are kept | proposed | TAG L147-168 | v8: a flag bit; fingerprint the grammar in the header |
| E28 | Sidecars `t_nidx`/`t_fidx`/`t_vpos` (rows per node, field edge, posting) | SQLite 211.3 of 240.4 B/node (88%); MariaDB 456 of 490 (93%); bought latency only | **deleted** (D14); enforced by a test | D S089 L4859-4959; DH L2408-2421 | DLRE's `decoded_*` caches are this shape. Fixed-width v8 columns remove the reason for them |
| E29 | Chunk the tape into a side table | +0.41 B/node, worse with `nera_tape` | rejected | D S067 L3875-3898 | v8 keeps one row (or object) per document |
| E30 | Per-touch cost decomposition (PostgreSQL) | 110 ns/touch = 51 row machinery + 21 slicing + 38 kind decode (+4 de-TOAST); better encoding capped at about half | encoding is not the main lever **in SQL** | DH L2599-2615 (S152); D S151-S152 | In a JIT/GPU executor the 51 ns of row machinery disappears, so decode cost becomes the main cost. **Regular decode matters more for v8's executors than it did for the spike's SQL** |
| E31 | Native TVF/UDF boundary: take an address, not the value; per-call cost | `nera_tape` 153 ns/call, ~120 ns of it dispatch; passing the tape by value rebuilt it per row; fix: carry `rec` as a column / blob handle; `sqlite3_blob_reopen` nearly free | adopted | DH L5973-5987 (S220), L5997-5999 | v8 vtabs take `(doc, column)` handles and slice by stride. **Fixed stride is what lets a vtab return a whole run of rows per call** |
| E32 | `nera_kids` child vtab; `nera_spans` bulk vs point | TS 21.8 s → 55 ms; point span 0.18-0.33 µs flat vs bulk 0.015 µs/node; the plan must choose | adopted later | DH L1698-1713, L2148-2151, L4970-4978 | with fixed spans a point span is a single slice; the bulk/point choice disappears |
| E33 | kidx absolute vs delta preorders | 10 ms gain, 2.6 → 4.2 B/node | reverted (D10) | DH L1546-1552 | posting lists may stay delta/varint: they are scanned sequentially, not randomly accessed. **Regularity is required for per-node columns, not for postings** |
| E34 | Per-engine physical storage | tape 5.21 PG, 14.73 SQLite, 6.13 MariaDB (incl. source); logical tree encoding **8.66 B/node** (6 rec + ~2.3 spans + ~0.4 seek/flags), 44% over the 6 B budget; SQLite does not compress | budget missed, recorded | R L900-945; SORA 4b | The 6 B budget was missed even with varint spans. v8 for normal documents is ~11 B/node logical (+2.3 over 8.66 once spans become 4 B fixed and flags 1 B). That is the price of full regularity; at rest zstd recovers most of it (E16: fixed records compress about 10x) |
| E35 | Edge index (parent kind, field, child kind) vs aggregate `efreq` | edge index buys nothing; efreq 0.039 B/node | aggregate only (D21) | DH L4933, L8691-8730 | statistics belong in aggregates, not per-node columns |

**Where Rerail and the spike differ, and which choice the measurements support:**

| Rerail choice | Spike choice | Better supported by measurement |
|---|---|---|
| extra/missing/error as sorted per-file lists | extra in the kind dictionary, missing sparse | Spike for extra (measured uniform per kind). For a GPU or SQL reader, DLRE's 1-byte flags column is better than either |
| Aliases as per-file strings plus overrides | interned codes, `code_ctx` spare codes | Spike: strings per file cannot be decoded per node without a lookup |
| Local kind tokens in the content-store FTS | global anchor codes (B) | Spike, measured −5 to −22% and exact over-scan removal. Also required for correct per-language statistics |
| v4/v5 multiple grammars in one tape | separate tapes (rejected splice) | Spike; Rerail converged in v7 |
| v7 generated-text documents with disjoint maps | contiguous segments | Rerail: the spike never handled generated text, and v7's model is the generalization |
| K=16 varint spans | K=16 varint spans | Same, but the spike's sweep trend (E18) and E13 favor fixed width for any consumer that touches spans per node |

## 5. A regular v8 candidate

**Goal:** every per-node field of node *i* is `column[i·w]`, where *w* is fixed for the document. There are no escape tables, no varints, no checkpoints and no per-node lookups. Documents are explicit.

**Layout of one document object.** It is content-addressed by (source digest, grammar fingerprint, encoder version).

```
header (fixed, 64 B):
  magic "RRTAPE8\0", version u32, lang u16, grammar_fingerprint [16], encoder u16,
  node_count u32, source_len u32, width_flags u8   (bit0: wide nodes (u32) if node_count > 65536;
                                                    bit1: wide spans (u32) if source_len > 65535),
  column directory: (offset u32, length u32) × C
columns (structure of arrays; each aligned to 8; little-endian):
  kf      u16 × n   kind10 | field6<<10. kind is a *document code* in the grammar's extended
                    code space (see aliases); ERROR has a reserved code (e.g. 1023) instead of
                    "code 0 + list"
  par     u16|u32 × n  i - parent (root: 0 or i+1, fixed convention); width per width_flags bit0
  size    u16|u32 × n  subtree node count; width per bit0
  start   u16|u32 × n  absolute byte start; width per bit1
  len     u16|u32 × n  byte length; width per bit1
  flags   u8 × n    named | extra | missing | error | first_field | (3 spare)
optional columns (present iff listed in the directory; same stride rule):
  lines   u32 × L   line-start byte offsets (L = line count), replaces per-restore LineTable scan
  ordinal u16|u32 × n  child ordinal (for SQL consumers that need sibling positions)
  next_named u16|u32 × n  (executor convenience; otherwise built in one O(n) pass)
```

**Why each piece:**

- **Width per document, not per node.** It can only be wide when `node_count > 65,536` (E14 numbers: par-ovf 0.31% and size-ovf 32 rows came only from very large files). Branching once per document is uniform across a GPU threadgroup and free in SQL, because the width is a constant in the generated statement.
- **Absolute fixed start/len.** O(1) spans, and binary search on monotone `start` for viewports and byte → node lookups (E19). Ends the checkpoint machinery and its 16 MiB `CHECKPOINT_LIMIT`.
- **One flags byte.** DLRE's index already does this. It replaces three sorted lists.
- **Aliases and ERROR as codes.** Contextual alias names (tree-sitter alias sequences) are static per grammar, so they are enumerated at **grammar registration** into spare codes of the 10-bit space (E21, E23). The grammar table maps each code to (display name, named, native `kind_id`), so matcher parity keeps the native id.
  - This must be verified: catalog plus alias count stays ≤ 1023 for every Rerail grammar (the spike's maximum was 582 visible kinds).
  - Fallback if a grammar exceeds 10 bits: widen `kf` to u32 for that grammar only (width flag bit2), never a per-file string table.
- **Supertypes (Tempo).** Supertype membership by kind is grammar-static. Context-dependent supertype paths, if still needed, become an optional regular column, not a pair list.

**Multi-document:** as in section 3.

- A file record holds a fixed-width document table and regular arrays for maps and host errors.
- Each document is a v8 object as above.
- Batches concatenate columns and use a batch document table (`node_base`, `text_base`, `line_base`, `lang`).

**Storage, using spike numbers as the yardstick:**

- **Normal document** (n ≤ 65,536 and source < 64 KiB; the median spike file was 599 nodes):
  - 2 + 2 + 2 + 2 + 2 + 1 = **11 B/node** logical, against 8.66 for the spike's v2 or Rerail v7;
  - plus optional `lines` (~4 B per line, about 0.1-0.3 B/node).
- **Large document:** 2 + 4 + 4 + 4 + 4 + 1 = **19 B/node** (rare).
- **At rest:** zstd per object, as the content store does now. E16 shows fixed-width columns compress about 10x (7.37 → 0.70 B/node pglz for the 7-byte record; spans 24.4 → 4.8 MB), and SoA columns compress well under zstd. The at-rest cost difference against v7 is expected to be small. **Measure it on the Rerail corpus before adopting.**
- **For random access** (SQL vtab, GPU upload) the uncompressed columns are used directly: mmap or memcpy, no restore loop.
  - On SQLite, an uncompressed per-document column store costs about 11 B/node, against the spike's physical 14.73 B/node *including source* for v2. That is comparable, because v8 drops checkpoints and overflow rows.

**What must change:**

- **Writer.**
  - Choose widths per document.
  - Emit absolute spans.
  - Emit a flags byte.
  - Map ERROR to its reserved code and aliases to registration-time codes. Grammar-registration work: enumerate alias sequences, assign codes, and fingerprint the result.
  - Emit the document table, regular map triples and host-error pairs.
  - Optionally emit line starts.
- **Readers.**
  - `TapeDoc` restore becomes column views plus an optional O(n) pass for derived navigation (next/prev/first/child count) or reads optional columns. The `Rec` rebuild becomes lazy.
  - `wire.rs::index_span` and the checkpoint code are deleted.
  - DLRE's anchor store drops `spans`/`checkpoints`/`parb`/`sizb`/`parovf`/`sizeovf` and the `decoded_*` caches, and reads `start`/`len`/`par`/`size` by stride.
  - N1562's `MetalNode` packing can upload columns directly. Its derived fields (`next_named`, `child_start`/`child_count`, `descendant_end = i + size`) are computed on the device in one pass, or by the CPU as an optional column.
- **Index and statistics.** Global kind tokens `lang·1024 + local` in `search_kind_fts` (Option B), so `fts5vocab` statistics are per language.
- **Migration.** Bump the version and use a distinct magic, so it never collides with Tempo's or Rerail's `TMPTAPE*`. Re-encode from the stored source; parsing is cheap relative to publication (the spike: parsing 1.9% of build time).

**Things in v7 that are really needed and must be carried regularly:**

| v7 feature | v8 |
|---|---|
| contextual aliases | registration-time codes |
| ERROR, MISSING, extra | reserved code and flag bits |
| per-document `host_errors` | regular pair array |
| disjoint source maps | regular triples |
| derived-document role, via, parent | document table columns |
| source digest | header, plus a grammar fingerprint (TAG L159-168) |

**Open measurements before locking v8:**

1. Alias and code-space counts per Rerail grammar (≤ 1023?).
2. Whether `extra` is uniform per kind in Rerail's grammars (TAG measured 18.5M nodes uniform).
3. At-rest zstd size of v8 against v7 on the full data-layer repository and the CPython corpus.
4. GPU: column upload against `MetalNode` expansion, end-to-end on the per-edit gate.
5. The width distribution: share of documents above 65,536 nodes or above 64 KiB of source.
