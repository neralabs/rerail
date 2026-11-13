# 0134: JSON is read with simdjson

Status: accepted (user, 2026-11-12: "please use the real SIMDJSON parser instead of tree-sitter-json
or bitlex-json. The tree-sitter one is slow and the bitlex one is not ready")

## Intro

A JSON file's tape was tree-sitter-json's tree, encoded. The Bitlex parser
(`transducer/packed/json_parser.rs`, the ladder's JSONC grammar compiled to tier 1 and tier 2)
gives the same tape, and was registered as JSON's parser, but it is not ready for the product.

Now the engine reads JSON with simdjson itself: the C++ library, vendored, validating the whole
file with its DOM parser. The tree is read from simdjson's structural index, so the tape is
tree-sitter-json's, node for node.

```
before: source -> tree-sitter-json -> tree -> tape        (34.8 G instructions to parse)
after:  source -> simdjson (DOM: validate; stage 1 index) -> tree -> tape
                                                           (1.1 G to parse and build the tree)
        a file simdjson refuses -> tree-sitter-json, as before
```

## Prior work consulted

- **ast-index-spike indexes:** the spike parsed with tree-sitter alone and has no JSON
  parser to compare; its rule of measuring against the real reference holds (0069: simdjson's own
  timings on this machine).
- **Lessons ledger #28** (only end-to-end CPU time counts). **Agrees:** the measure is the
  product's path, source to tape (`tape::tests::json_parse_costs`), not the parse alone.
- **Lessons ledger #10** (measure the code that runs). **Agrees:** the oracle compares the tapes
  the engine stores, byte for byte.
- **0069** (compiled languages keep simdjson's shape). **Unchanged:** the Bitlex parser stays,
  with its tests, as the compiler's JSON target; it is no longer the product's JSON parser.
- **docs/parser-generators/21** (JSON against simdjson). Its `simdjson_check` tool linked
  Homebrew's simdjson; the engine now builds the same release (5.0.5) from source.

## Decision

1. **simdjson is vendored**, unchanged: the 5.0.5 amalgamation, `native/vendor/simdjson-5.0.5`
   (Apache-2.0 or MIT; its README gives the source and hashes). The crate `rerail-simdjson`
   (`native/crates/simdjson`) compiles it with `cc`, at `-O3` in every profile, with a shim:
   - one DOM parser and padded buffer per thread; a thread's buffers are freed after a file over
     1 MB (the DOM parser holds about 14 bytes a source byte);
   - `structurals(src, read)`: the parse, then the structural index (`{ } [ ] : ,` and each
     scalar's first byte), lent to `read`.
2. **The tape is written straight from simdjson's index**, in Rust in that crate
   (`rerail_simdjson::tape`), optimized as a dependency is (`[profile.dev.package.rerail-simdjson]`).
   One pass writes each column at the node's preorder slot, as doc 13 §2 describes:
   - at a node's start: its code (numbered by first use), its offset from its block's base, and a
     base for every 16 nodes;
   - at its end: its size (the nodes written since it) and its tail (its end less the last
     descendant's start);
   - `size`, `off` and `tail` are narrowed to 2 bytes at the end where the file's largest fits.

   What the index leaves to the writer:
   - a string's closing quote and a scalar's end are the last byte before the next index that
     is not whitespace;
   - a string's escapes are found with `memchr`; each is two bytes (tree-sitter-json's
     `escape_sequence`), and its content is the runs between them;
   - the document runs from its first token to the end of the file, as tree-sitter's root;
   - offsets after a byte order mark are shifted by its 3 bytes (simdjson skips it);
   - the `_value` supertype marks the document's child, array elements and pair values.

   The engine takes the columns as written (`TapeImage::written`: their lengths checked, the
   source's digest added). No `Pending` record is made, and `TapeImage::finish` does not run.
3. **A file simdjson refuses goes to tree-sitter-json:** comments (JSONC), a trailing comma, a
   number past simdjson's range, nesting past 1,024, invalid JSON. Its tape is what it was.
4. **The oracle** (`json_tapes_from_simdjson_are_tree_sitters`): hand-written cases (escapes,
   surrogates, empty strings and keys, a BOM, CRLF, nesting), and every `.json` file of the
   repository up to 1 MB. Each tape equals tree-sitter-json's byte for byte, except where
   tree-sitter-json finds an error in valid JSON (an exponent's `+`), where it must equal the
   Bitlex parser's.
5. **wasm** builds no C++: there JSON goes to tree-sitter-json.

## Results

Every `.json` file of the repository up to 1 MB that both simdjson and the Bitlex parser accept (2,200
files, 73.6 MB, 7.2 M nodes), debug build (the engine unoptimized, dependencies at `-O3`), least of 3
rounds (`tape::tests::json_parse_costs`):

| | instructions | a node | CPU |
|---|---:|---:|---:|
| simdjson: parse and index | 0.45 G | 62 | 25 ms |
| simdjson: parse, index and the columns | 1.63 G | 227 | 73 ms |
| **simdjson: the tape** (the product's path) | **1.89 G** | 263 | 99 ms |
| Bitlex: parse (its node table) | 3.35 G | 465 | 178 ms |
| Bitlex: the tape (node table, then `finish`) | 35.6 G | 4,937 | 1.80 s |
| tree-sitter-json: parse | 34.8 G | 4,832 | 1.43 s |
| tree-sitter-json: the tape (before) | 130.3 G | 18,089 | 6.96 s |

The tape is the columns plus the source's SHA-256 (0.21 G). 2,199 tapes equal tree-sitter-json's
byte for byte; 1 is tree-sitter's error (`7e+308`); 1 file is refused.

The first build made a tree, then packed it as tree-sitter's trees are packed: 32.6 G, of which
31.5 G was `TapeImage::finish` (generic, so compiled unoptimized in the parser's crate). The
Bitlex path still packs that way (O-bitlex-writes-tape); tree-sitter's trees do too
(O-tape-pack-cost).

The Bitlex parse is 85% generated code (tier 2 51%, tier 1 34%), the rest buffer set-up.
