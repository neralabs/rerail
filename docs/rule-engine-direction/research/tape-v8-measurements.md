# Tape v8 spike: content audit, Blade, measured variants, SQL test

Spike code: `crate/` (v7 encoder and decoder copied from `native/src/code_rule_dsl/tape.rs` and `tape/wire.rs`; v8 and v9 variants beside it). Raw results: `results/*.json`, `results/sql*.txt`, `results/tables.md`. Corpora: CPython 3.13.0 `Lib/` (sparse clone in `corpora/`), nera-data-layer `impl/` (gruel-main checkout), rerail TS (`src test conformance benchmark scripts`), rerail Rust (`native/src crates`), rerail PHP fixtures (non-Blade `.php` under `fixtures packs`). 24.2M nodes in total. Grammars: the crates.io tree-sitter grammars (python 0.25.0, typescript 0.23.2, rust 0.24.2, php 0.24.2) on tree-sitter 0.27.1, not Rerail's vendored and patched ones (see Caveats).

## Recommendation

1. **Do not adopt v8 as proposed.** On real corpora it is 11.1–13.5 B/node logical (1.24–1.59× v7) and 1.47–1.90× v7 after zstd. Large files cause it: 124 of CPython's 1,702 files are 64 KiB or larger but hold 47% of its nodes, and they force u32 absolute `start`/`len`. Absolute positions also compress badly; v7's delta varints compress well.
2. **Adopt "v9" instead: regular O(1) access with no absolute positions**, measured below:
   - `start(i) = base[i/16] + off[i]`: a u32 base per 16-node block plus a u16 offset (preorder starts are non-decreasing).
   - `end(i) = start(i+size(i)-1) + tail[i]`: the end measured from the start of the node's last descendant (for a leaf, its length).
   - `kf` u16 (10-bit kind, 6-bit field) with **flags folded into the kind code space** (named, extra, missing, error, contextual aliases as distinct codes); `size` u16/u32 per document; parent delta optional.
   - Across 24M nodes no document needed 32-bit `off` or `tail`, so the only per-document width switch left is `size`/`par` (files with ≥65,535 nodes: 24 of 6,251).
3. **Size:** v9 without the parent column ("v9np", columns `kf, size, base, off, tail`) is **8.25–8.55 B/node logical (0.93–1.01× v7) and 0.94–1.06× v7 after zstd**. The 8→11 B concern disappears. With the parent column (v9) it is 10.25–10.84 B/node (1.15–1.28×; zstd 1.13–1.28×).
4. **Recommended at rest: v9np (SoA columns), plus a parent column derived when a consumer needs it** (4.1–5.1 ns/node to rebuild with a stack, the same cost as today's v7 restore of 3.7–7.8 ns/node, but paid only where parents are used). Keep the parent column only if SQL parent climbs (`inside`/`has` with `stopBy: neighbor`, `follows`) prove hot. In SQL a climb was 1.6–3× faster than interval containment (table below).
5. **The clear benefits are random access and spans**, not scans:
   - Rust random access to (kind, parent, size, span, named): v7 113–490 ns, v9 23–199 ns, v8 10–96 ns.
   - Random span alone: v7 33–364 ns, v9 13–57 ns, v8 3.5–13 ns.
   - Sequential scans are equal (v7 3.8–4.3 ns/node, v8 2.9–3.8) or worse for v9 (8.8–24 ns/node) when every end is recomputed from the last descendant's start. A sequential walker should carry ends on a stack instead; that was not measured.
   - SQL: span extraction is 2.7–8× faster than v7 (below). Structure queries are not faster, because pure-SQL time is dominated by blob reloads, not decoding.
   - JIT and Metal: columns upload as they are, with no per-batch restore into a 44-byte `MetalNode`. v7's restore cost is 3.7–7.8 ns/node. A derived parent column, if needed, is the same order of cost.

## Part 1: tape content audit

What v7 writes (`TapeImage::to_bytes`) and what the store keeps beside it:

| Item | Where | (a) structure / (b) associated | Note |
|---|---|---|---|
| kind (10 bits), field (6 bits) | `rec` | a | |
| parent delta, subtree size (u16, `0xFFFF` → overflow pairs) | `rec` + `par_overflow`, `size_overflow` | a | overflow is 0.011–0.046 B/node of side tables in total, with all flag lists |
| start, length | varint `spans` + `checkpoints` every 16 nodes | a | 2.41–2.42 B/node |
| extra, error, missing | sorted u32 lists | a | rare (error 0–83 nodes per corpus; missing 0 in these corpora) |
| named | not stored; static catalog unless overridden | a | |
| contextual kind aliases | per-file string table + `(pre, alias)` overrides | **b** (grammar property, duplicated per file) | 0 aliases in the crates.io grammars; Rerail's patched grammars must be measured |
| language tag, source length, SHA-256 digest, 11 table lengths | 144-byte header | **b** (identity) | the digest is the store's key; it does not need to be in the tape |
| derived documents (text, source map, host errors, role, via) | store rows `file_documents` (`schema.rs:92`), each derived tape its own `tape_objects` row | **b** | already outside the tape in v7 |
| Blade compiled-view omissions | derived tape built with `omitted` segments: nodes over Blade's synthetic placeholders are dropped (`encode_tree_image(.., Some(&segments))`) | **b leaking into a** | that tape is *not* what tree-sitter-php produced |
| line table | recomputed on restore | b | correct already |

**Minimal structure-only tape (proposed).** Exactly the parser's tree: `kf` (folded codes), `size`, `base`/`off`/`tail`, optionally the parent, a per-document width byte and the node count. Everything in (b) goes to sidecars keyed by the tape's content digest, or is recomputed:
- **Kind aliases** are assigned codes once per grammar at registration (the spike's spare-code technique). The folded-code count per grammar is 194–277 of 1,024, so there is room.
- **The digest and source length** live in the store row.
- **Derived-document maps and host errors** stay in `file_documents`, as now.
- **The Blade view** is a derivation sidecar (below).

## Part 1b: Blade

History: Blade was parsed with tree-sitter-blade until `aac1ff23` (2026-10-13, "Parse Blade with a port of Forte 1.1.0 instead of tree-sitter-blade"). Before v7, the Blade tape carried the PHP it embeds with kind codes shifted by an offset (`BLADE_EMBEDDED`, v4), then grafted programs (v5), then nested guests (v6). The compiled-view program was introduced as a tree-sitter-php parse in `c5128c84` (2026-10-14).

What a Blade file stores today (v7):
1. **The primary tape**: Forte's tree in Blade's own kind table, plus **synthetic zero-width ERROR leaves** inserted at the positions of Forte's lexer errors (`encode_blade_tree`). These nodes are not in Forte's tree.
2. **One derived document (`compiled-view`)**: the PHP text Rerail writes in place of Blade's compiler output, its segment map, host errors, and a **tree-sitter-php tape of that text with the nodes over synthetic placeholder segments removed** (`omitted`). As a result a directive call "has no `function` child" (`blade/view.rs` header). The tape is less than tree-sitter-php produced, and its parent and size values describe a pruned tree.

Proposal, so that Blade stores nothing more or less than the tree-sitter-php tape it replaced:
- **The compiled-view document's tape is the plain tree-sitter-php tape of the view text, with no omission.** The omit flags already live in the segment map (`SegmentMap` flags bit 2). "Skip nodes over synthetic text" becomes a matcher and evidence concern applied through the sidecar, not a property of the tape.
- **The view text, segment map and host errors stay a derivation sidecar** keyed by (template digest, via). They are deterministic and recomputable from the template.
- **Forte errors:** keep the Forte tree as its own tape, but drop the synthetic ERROR leaves. Lexer-error ranges become a sidecar (they were the v4 `syntax_errors` table). Rules that need "parse error here" read the sidecar the same way the host-errors path does.
- Not measured here: the Forte port is not in the spike. The size effect is tiny either way (error leaves and omitted nodes are a small fraction of a template's nodes).

## Part 2/3: variants and measurements

Variants (all encoded from the same node list; decoders checked against each other: `parity True` on every corpus for random and sequential checksums):

| Variant | Layout | B/node at u16 widths |
|---|---|---|
| v7 | rec6 + varint spans + checkpoints + overflow and flag lists + alias tables (copied encoder) | ~8.4 |
| v8 | SoA `kf, par, size, start, len, flags`, per-document u16/u32 widths | 11 (13 with u32 spans) |
| v8i | v8 interleaved per node | same |
| v8k | v8 with flags folded into kind codes | 10 |
| v8np | v8k without the parent | 8 (10 with u32 spans) |
| v9 | SoA `kf, par, size, base, off, tail`, flags folded | 10.25 |
| v9np | v9 without the parent | 8.25 |
| v9r | v7's rec6 (kf, par, size) unchanged + interleaved (off, tail) + base | 10.25 |
| v9i | interleaved (kf, size, off, tail) + base | 8.25 |

| corpus | files | nodes | source B/node | source zstd B/node |
|---|---|---|---|---|
| cpython-3.13-Lib | 1702 | 7,897,149 | 4.18 | 0.97 |
| nera-data-layer-impl | 3043 | 11,619,212 | 5.03 | 1.02 |
| rerail-ts | 559 | 2,029,743 | 3.44 | 1.04 |
| rerail-rust | 413 | 2,454,794 | 3.56 | 0.96 |
| rerail-php-fixtures | 534 | 161,595 | 3.26 | 1.23 |

Logical B/node

| corpus | v7 | v8 | v8i | v8k | v8np | v9 | v9np | v9r | v9i |
|---|---|---|---|---|---|---|---|---|---|
| cpython-3.13-Lib | 8.49 | 13.12 | 13.12 | 12.12 | 10.00 | 10.50 | 8.38 | 10.50 | 8.38 |
| nera-data-layer-impl | 8.47 | 13.50 | 13.50 | 12.50 | 10.21 | 10.84 | 8.55 | 10.84 | 8.55 |
| rerail-ts | 8.47 | 11.69 | 11.69 | 10.69 | 8.62 | 10.39 | 8.32 | 10.39 | 8.32 |
| rerail-rust | 8.47 | 11.82 | 11.82 | 10.82 | 8.82 | 10.25 | 8.25 | 10.25 | 8.25 |
| rerail-php-fixtures | 8.93 | 11.05 | 11.05 | 10.05 | 8.05 | 10.31 | 8.31 | 10.31 | 8.31 |

zstd -3 B/node (per document)

| corpus | v7 | v8 | v8i | v8k | v8np | v9 | v9np | v9r | v9i |
|---|---|---|---|---|---|---|---|---|---|
| cpython-3.13-Lib | 1.74 | 3.29 | 3.67 | 3.14 | 2.77 | 2.15 | 1.77 | 2.10 | 2.14 |
| nera-data-layer-impl | 1.85 | 3.48 | 3.89 | 3.33 | 2.94 | 2.36 | 1.97 | 2.34 | 2.32 |
| rerail-ts | 2.18 | 3.80 | 4.03 | 3.63 | 3.17 | 2.50 | 2.05 | 2.49 | 2.43 |
| rerail-rust | 2.07 | 3.72 | 3.98 | 3.54 | 3.10 | 2.46 | 2.01 | 2.43 | 2.39 |
| rerail-php-fixtures | 2.69 | 3.96 | 4.28 | 3.77 | 3.25 | 3.04 | 2.53 | 3.10 | 2.85 |

Ratio to v7 (logical / zstd)

| corpus | v8 | v8i | v8k | v8np | v9 | v9np | v9r | v9i |
|---|---|---|---|---|---|---|---|---|
| cpython-3.13-Lib | 1.55 / 1.90 | 1.55 / 2.11 | 1.43 / 1.81 | 1.18 / 1.60 | 1.24 / 1.24 | 0.99 / 1.02 | 1.24 / 1.21 | 0.99 / 1.23 |
| nera-data-layer-impl | 1.59 / 1.88 | 1.59 / 2.10 | 1.48 / 1.79 | 1.21 / 1.58 | 1.28 / 1.28 | 1.01 / 1.06 | 1.28 / 1.26 | 1.01 / 1.25 |
| rerail-ts | 1.38 / 1.74 | 1.38 / 1.85 | 1.26 / 1.67 | 1.02 / 1.46 | 1.23 / 1.15 | 0.98 / 0.94 | 1.23 / 1.14 | 0.98 / 1.12 |
| rerail-rust | 1.40 / 1.79 | 1.40 / 1.92 | 1.28 / 1.71 | 1.04 / 1.49 | 1.21 / 1.19 | 0.97 / 0.97 | 1.21 / 1.17 | 0.97 / 1.15 |
| rerail-php-fixtures | 1.24 / 1.47 | 1.24 / 1.59 | 1.13 / 1.40 | 0.90 / 1.20 | 1.15 / 1.13 | 0.93 / 0.94 | 1.15 / 1.15 | 0.93 / 1.06 |

Rust decode, ns (random: per probe of kind+parent+size+span+named over 2M random nodes; seq: per node)

| corpus | random_v7 | random_v8 | random_v9 | random_span_v7 | random_span_v8 | random_span_v9 | seq_v7 | seq_v8 | seq_v9 | v7_restore_to_u32x5 | v9np_parent_rebuild | parity |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| cpython-3.13-Lib | 245.1 | 41.0 | 78.5 | 222.8 | 11.9 | 45.9 | 3.9 | 3.8 | 8.8 | 3.7 | 4.2 | True |
| nera-data-layer-impl | 489.8 | 96.0 | 198.5 | 364.4 | 13.3 | 56.5 | 4.3 | 3.8 | 24.3 | 7.8 | 4.1 | True |
| rerail-ts | 179.8 | 73.3 | 93.8 | 78.1 | 6.9 | 47.6 | 4.0 | 3.2 | 9.8 | 3.8 | 4.6 | True |
| rerail-rust | 258.4 | 76.8 | 91.9 | 183.3 | 8.3 | 37.4 | 3.8 | 3.1 | 9.2 | 3.8 | 4.5 | True |
| rerail-php-fixtures | 113.4 | 10.4 | 23.2 | 32.7 | 3.5 | 13.0 | 3.9 | 2.9 | 9.6 | 4.0 | 5.1 | True |

Widths, code space, invariants

| corpus | docs | docs u32 par/size | docs u32 start/len (v8) | nodes in those | v9 docs u32 off/tail | max kind / catalog | max field | folded codes | aliases | non-uniform extra kinds (minority nodes) | extra | missing | error | internal | end≠end(last) | start≠start(first) | leaves ≥256 B | v7 side tables B/node |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| cpython-3.13-Lib | 1702 | 6 | 124 | 3,703,217 | 0/0 | 273/274 | 32 | [209] | 0 | 1 (1) | 91021 | 0 | 13 | 2,672,175 | 8171 (0.31%) | 19596 | 2720 | 0.046 |
| nera-data-layer-impl | 3043 | 17 | 171 | 5,564,877 | 0/0 | 272/274 | 32 | [194] | 0 | 0 (0) | 26062 | 0 | 83 | 3,925,410 | 3944 (0.10%) | 1067 | 3588 | 0.011 |
| rerail-ts | 559 | 1 | 9 | 279,160 | 0/0 | 382/383 | 40 | [257] | 0 | 1 (2) | 6784 | 0 | 5 | 657,612 | 559 (0.09%) | 0 | 904 | 0.013 |
| rerail-rust | 413 | 0 | 18 | 500,634 | 0/0 | 354/355 | 31 | [239] | 0 | 0 (0) | 16691 | 0 | 0 | 758,083 | 4758 (0.63%) | 326 | 85 | 0.027 |
| rerail-php-fixtures | 534 | 0 | 0 | 0 | 0/0 | 431/432 | 31 | [277] | 0 | 0 (0) | 1518 | 0 | 7 | 58,982 | 265 (0.45%) | 28 | 12 | 0.038 |

Encode ns/node (after parse; noisy under load)

| corpus | parse ns/node | v7 | v8 | v9 |
|---|---|---|---|---|
| cpython-3.13-Lib | 1304 | 26.7 | 34.5 | 39.0 |
| nera-data-layer-impl | 1575 | 38.2 | 17.9 | 50.4 |
| rerail-ts | 1044 | 11.4 | 9.1 | 11.7 |
| rerail-rust | 904 | 6.4 | 77.3 | 29.3 |
| rerail-php-fixtures | 1671 | 10.2 | 14.4 | 15.0 |

Observations:
- **Interleaving (v8i, v9i) compresses 6–21% worse than columns.** Columns are the at-rest layout.
- **Folding flags into the kind code is safe.** It needs 194–277 codes per grammar (limit 1,024). `extra` is uniform per kind except 1–2 nodes per corpus, and those nodes simply get their own code.
- **A derived end without a tail column would need an exception table.** `end ≠ end(last descendant)` for 0.09–0.63% of internal nodes, and only 0–2% of those have a special tail (extra, missing, error, zero width). The `tail` column makes the end exact and regular at 2 B/node. Likewise `start ≠ start(first child)` for up to 0.73%, so start is not derived.
- In documents of 64 KiB or more, `start − start(parent)` fits 16 bits for 99.8% of nodes. That was not used, because a climb is needed to read it.

## Part 4: SQL (ast-index-spike pure-SQL technique, SQLite 3.53.2 bundled)

Setup:
- Per-file blobs; a byte is `instr(X'00..FF', substr(col, pos, 1)) - 1` (no registered function); rows come from `generate_series`.
- 387 CPython files under 64 KiB (13 of the first 400 excluded so that every table is u16-wide and v7's overflow tables are empty); 809,912 nodes.
- v7 spans are decoded in SQL by a recursive CTE that walks the varints byte by byte from the 16-node checkpoint, exactly as `wire::index_span` does.
- Every answer matches the Rust oracle (`parity` column).
- Times are the minimum of 3 runs on a machine with load average 50–65, so differences under about 1.5× are noise.

Run 3 (all layouts):

| query | v7 ms | v8 (SoA) ms | v9 (SoA) ms | v9r (v7 rec6 + span rec) ms | v9i (interleaved, no parent) ms | parity v7/v8/v9/v9r/v9i | answer |
|---|---|---|---|---|---|---|---|
| has: function_definition has return_statement (stopBy end) | 578.2 | 1059.0 | 994.7 | 625.9 | 541.9 | true/true/true/true/true | 3050 |
| nthChild 2 of call (sibling chain) | 5381.2 | 1638.9 | 1758.3 | 6989.0 | 6418.0 | true/true/true/true/true | 100,1;157,23263;223,100 |
| inside: return_statement inside function_definition (parent climb) | 551.5 | 978.2 | 893.0 | 894.0 | n/a | true/true/true/true/- | 4225 |
| inside as interval containment (no parent column) | 1652.5 | 2519.4 | 2463.4 | 1421.5 | 1307.5 | true/true/true/true/true | 4225 |
| span text of every string node | 9475.3 | 1193.5 | 2355.5 | 3517.7 | 1702.5 | true/true/true/true/true | 37582,841933,378163282 |

Runs 1 and 2 (v7, v8, v9 SoA), the same 387 files: has 444/890/814 and 483/1211/1394 ms; nthChild 3638/1183/1187 and 4910/1371/1407; climb 608/788/921 and 531/998/831; interval 810/1505/1416 and 1005/1916/1811; spans 6212/1100/2156 and 8810/1076/2507.

What the SQL numbers say:
1. **Spans: the regular formats win clearly.** v7 6.2–9.5 s; v8 1.1–1.2 s (5.6–8×); v9 SoA 2.2–2.5 s (2.7–3.8×); v9i 1.7 s (5.6×); v9r 3.5 s (2.7×). v9 needs 16 byte decodes per span against v8's 4. Each byte is one `instr(substr())`, and that is the whole cost in pure SQL.
2. **Structure queries depend on which blob SQLite reloads, not on whether the format is regular.**
   - When the anchor row is loaded once (`has`), the interleaved records (v7, v9r, v9i) are about 2× faster than SoA, because one blob holds kind and size.
   - When a recursive CTE re-joins the file row on every step (`nthChild`), SoA is 3–4× faster, because each step reloads only the 2-byte-per-node `size`/`kf` blobs, not a 6–8-byte-per-node record.
   - This is the spike's finding that per-node blob re-reads dominate. Kind, parent and size decoding is identical in v7 and v9, since v7's rec6 is already regular apart from overflow.
3. **Parent climb vs interval containment:** a climb over the parent column is 1.6–3× faster than an interval-containment join. Dropping the parent costs SQL `inside` about 2×.
4. **Honest answer to "is v7 as good in SQL?"** For structure (kind, parent, size), yes: v7's record is the spike's regular rec6. For positions, no: varint spans make every span 6–9× more expensive in pure SQL. Flags and aliases in v7 need list lookups (not measured in SQL; binary search in a blob is not expressible without recursion).

## What changes in the writer and readers (v9np)

- **Writer.**
  - Compute sizes as now.
  - Emit `kf` with folded codes, per-document `size` width, `base` per 16 nodes, `off` (u16, or u32 when any block spans 64 KiB or more; never seen), and `tail` (u16/u32 per document; never seen u32).
  - Encode is cheaper than parse by 12–260× in every variant (parse 904–1,671 ns/node; encode 6–77 ns/node, noisy).
- **Readers.**
  - Delete the checkpoint walk, the overflow and flag binary searches, the alias string tables, and DLRE's `decoded_*` row caches.
  - `TapeDoc::restore` becomes optional. Where ast-grep's `Doc` needs first/next/prev links, they are derived in one pass with a stack, as now.
  - The parent is derived on load when needed.
- **SQL.**
  - Spans are plain arithmetic. Kinds and sizes are as before.
  - Store `kf` and `size` as their own blob columns (recursive queries reload only those), or interleave kind and size in one blob for single-pass anchor scans. Measure on the real anchored workload before choosing.
- **Global kind codes for the FTS anchor index** (`lang*1024 + folded code`) follow directly from the folded code space.

## Caveats

- **Grammars.** These are the crates.io grammars, not Rerail's vendored and patched ones. Kind counts, aliases and `missing` frequency must be re-measured on Rerail's grammars before locking the code space (0 aliases and 0 missing nodes were observed here).
- **Load.** Load average 50–70 for the whole run: Rust timings are the minimum of 9 interleaved runs, SQL the minimum of 3. Treat cross-corpus timing differences as noise and the within-run ratios as the signal.
- **SQL subset.** Files under 64 KiB only, so the v7 overflow and u32 paths were not exercised in SQL.
- **Blade.** Audited from code, not measured.
- **Sequential v9 decode** used the per-node `start(last)` formula. A stack-carried end, not implemented, should match v8.

## Reproduce

```
S=/private/tmp/claude-501/-Users-jeremy-orca-workspaces-rerail-Pulling-it-all-together/3adb237a-22e7-4e0c-9a7a-33e109d2ab63/scratchpad/v8spike
git clone --depth 1 --branch v3.13.0 --filter=blob:none --sparse https://github.com/python/cpython.git $S/corpora/cpython && (cd $S/corpora/cpython && git sparse-checkout set Lib)
(cd $S/crate && CARGO_TARGET_DIR=../target cargo build --release)
B=$S/target/release/v8spike; R=~/orca/workspaces/rerail/Pulling-it-all-together; D=~/orca/projects/nera-data-layer-gruel-main
$B measure cpython-3.13-Lib python $S/corpora/cpython/Lib > $S/results/cpython.json
$B measure nera-data-layer-impl python $D/impl > $S/results/ndl-impl.json
$B measure rerail-ts typescript $R/src $R/test $R/conformance $R/benchmark $R/scripts > $S/results/rerail-ts.json
$B measure rerail-rust rust $R/native/src $R/crates > $S/results/rerail-rust.json
$B measure rerail-php-fixtures php $R/fixtures $R/packs > $S/results/rerail-php.json
$B sql python $S/results/<new>.db 400 $S/corpora/cpython/Lib     # the database path must not exist
```
