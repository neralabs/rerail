# 0161: The tape format from 11 to 15: named-only tapes, and no stored digest

Status: recorded after the fact (2026-11-13). Built 2026-11-09 to 2026-11-11, in the
named-only tapes work (RER-131), without a decision record and without the user's review.
The user will revisit it on another branch. This record states what was done and why, so that
a change to it is made in one place and knowingly.

## Intro

The engine's tape is a byte string, the v9np image (decision 0025): a header, five columns of
node data, and a table of codes. The crate `rerail_tape_format` defines it, and its
`FORMAT_VERSION` says which layout an image has. Every parser that writes a tape and every
reader of a stored one must agree on it.

Between 2026-11-09 and 2026-11-11 the version went from 11 to 15 in three steps. Each step was
made inside other work and described only in its commit message and in notes t27 and t28. Two
branches written against format 11 then had to be ported when they were merged: the
interpreter's tape writer (e3776af9c) and tier 2's (2a2f6e642, aeff716b1). This record says
what each version changed, the reasons given, and what was not measured.

## What each version changed

| Version | Commit | Change |
|---|---|---|
| 11 | (before 2026-11-09) | Columns aligned to 16 bytes. Every tree-sitter node stored, anonymous tokens too. |
| 12 | 7deb10859 | Tapes stored named-only and restored whole on read. The source's SHA-256 leaves the header. |
| 13 | (none) | No commit names a format 13. |
| 14 | 1b9c360dc | Tokens the grammar implies are never written, in memory or at rest. |
| 15 | 051e30dea | A keyword or operator is left out only where its node's kind always implies it. |

The header, format 11 and format 15:

```
// Format 11: 96 bytes. Byte offsets.
0   magic            8 bytes, "TMPTAP9N"
8   language tag     1
9   widths           3   (size, off, tail; the tail's flag)
12  version          4
16  source length    8
24  source SHA-256   32
56  five lengths     40  (nodes, codes, code table bytes, aliases, alias table bytes)

// Format 15: 64 bytes. The same, without the digest; the lengths move up.
0   magic            8
8   language tag     1
9   widths           3
12  version          4
16  source length    8
24  five lengths     40
```

`rerail_tape_format::header` names these offsets. A writer or reader takes them from there, so
a later change moves them in one place.

### Named-only (12, 14, 15)

A tree-sitter tree has a node for every token, named or not: `(`, `,`, `def`, `+`. Most of the
anonymous ones hold nothing that the tree does not already say.

```python
# Source.
def f(a, b): return a + b
```

```
// Format 11: every node, anonymous tokens marked with '.
function_definition
  'def'  identifier(f)  parameters  ':'  block
                        '(' identifier(a) ',' identifier(b) ')'
  ...
// Format 15: the tokens the parent's kind implies are not written.
function_definition
  identifier(f)  parameters  block
                 identifier(a)  identifier(b)
  ...
```

- 12 stored tapes named-only and restored the full tree on read (t27's option A). The tree in
  memory, and every index keyed by preorder position, were unchanged.
- 14 stopped writing implied tokens at all. While it walks tree-sitter's tree, the encoder asks
  the grammar's automaton which children are implied (`tokens::TokenGrammar::stored_children`).
  The fold, token recognition, the token columns, the per-tape token index and Metal's token
  kernel went. Every derived store that numbers nodes was bumped with it.
- 15 narrowed what is left out: a keyword or operator only where its kind always implies it
  (`tokens::kept`). Where a token's presence carries meaning, it is stored: a slice's `:`, a
  union pattern's `_`. Python rules that had read a token as a substring of its parent's text
  were rewritten to read stored tokens or the source around a node (051e30dea).

### No stored digest (12)

Format 11 stored the SHA-256 of the source in each header. Format 12 dropped it. A restored
tape's digest is computed again from its source, which the store keeps beside it, and the
engine holds it in memory (`TapeImage::source_digest`). Parsers write no digest.

## Why

- **Named-only.** Anonymous tokens are 35–46% of all nodes (t27 §1, a model of v9np's columns
  over real trees). Tapes came to 0.54–0.65× the nodes and bytes, and 0.71–0.77× after zstd,
  by language. Every pass that walks a tape would visit that many fewer nodes: `inside`, `has`,
  `nearest`, kind postings, GPU slabs, the facts projection, the prefilter. simdjson writes no
  entry for `{`, `:` or `,` for the same reason. The named-only direction is RER-131's. The
  rule that no producer writes an implied token and then drops it is the user's (2026-11-12).
- **No digest.** 32 bytes a tape. On the Python store that is about 7% of compressed tape
  bytes, because most tapes are small: there the header, digest and code table are a third of
  the frame (t28 §3). The store already pairs each tape with its source.

## What was not measured

- **The walk speed-up.** t27 says it "has not been measured". The saving claimed for passes
  over tapes rests on node counts, not timings.
- **The digest's cost.** Restoring a stored tape now hashes its source again. That cost was
  not measured against the 7% of bytes saved, nor against keeping the digest in the store
  beside the tape rather than in it.
- **What 14 and 15 cost the readers.** Rules that read implied tokens were rewritten (Python's
  in 051e30dea). Pack rules testing implied tokens by regex (`^\(`, `^\.`), the readers of
  `core.rr`'s token relations and the anchor-index suites were listed as "not yet" in
  1b9c360dc.

## Consequences

- A format change goes in `rerail_tape_format` only, with its `header` offsets. Code written
  against an older version is ported when it is merged; `docs/merging.md` lists "one tape
  format, with no stored digest" among the invariants merges have undone before.
- If the digest returns, it returns in one place: `header`, `FORMAT_VERSION`, the `Writer`, and
  the header writers of tier 2 (`native_cpu_tape::finish`, `rust_stream`'s `accept`).
- Stored tapes are looked up by the format version with the grammar's (`tape_by_source`,
  `facts::families`): a tape of another version is not found, and its source is parsed again.
  There are no migrations; nothing is deployed.

## Prior work

- docs/final-shape/notes/t27-named-only-tapes.md: the assessment and plan, the step results.
- docs/final-shape/notes/t28-store-compression.md §3: the share of header and digest in a frame.
- docs/final-shape/notes/t30-named-only-in-memory.md, t31-token-readers.md: steps 14 and 15.
- Decision 0025 (v9np), 0140 (the parsers write the tape's bytes).
