# 0026: nodes live in tapes

Status: accepted (2026-11-02, user decision)

## Context

The plan kept per-node arrays beside the tape under the tape's digest and called them sidecars:
parent, position among siblings (all and named), line starts, the term id (`tid`) and text hashes
(03 §5 and §16, O7, the Phase 9 slabs row). The user's ruling: there are no per-node structures
anywhere in the design or the implementation. Nodes are stored in tapes, and the only other thing
over nodes that may be considered is a bare index: a key to the set of nodes that have it.

ast-index-spike is the reference (`notes/t4-ast-index-spike.md`):
- "Nothing may store one row per node" (§1.1; Representation rules 1–4; D S089 L4859-4959). Row per
  node measured 14.51× source on PG and 4.87× on SQLite.
- Persisted sidecars were dead end D14: `t_nidx` (a row per node: parent, kind, size, ordinal,
  depth, sibling status), `t_fidx` and `t_vpos` cost 211 B/node on SQLite, 88–93% of the store, and
  bought latency only. They were deleted and a test enforces their absence (DH L2408-2421; HI L294).
- Stored versus derived (§2.5): the tape stores kind/field, size and spans; parent, first child, next
  sibling and descendant are arithmetic on the tape; ordinals are "computed in the walk, and only
  for the parents a pattern compares" (global computation was 75× slower, D S037); depth was deleted.
- The useful node-access primitives are positional kind postings, value postings, the per-file tape
  with O(1) record access, and aggregate edge statistics. "Node-level edge or pair indexes were
  repeatedly not worth it" (HI L312-313).
- "Production can read rec6 directly" (§2.3; TAG L293-299): no materialised navigation.

## Decision

The tape (v9np, decision 0025) is the only per-node structure. Everything a consumer needs of a
node is read from the tape's columns in O(1), computed in the walk that visits it, or read from a
bare index. Nothing is materialised per node, stored or resident, on the CPU or the GPU.

**Access from the tape.** `i` is a node's preorder; the columns are `code`, `size`, `base` (one u32
per 16 nodes), `off` and `tail`.

| What | How it is read | Cost |
|---|---|---|
| kind, field, flags (extra, error, missing), alias | `codes[code[i]]` | O(1) |
| supertype memberships | `codes[code[i]].classes` | O(1) |
| start byte | `base[i/16] + off[i]` | O(1) |
| end byte | `start(i + size[i] − 1) + tail[i]` | O(1) |
| text | `source[start..end]` | O(1) |
| descendant test | `i < j < i + size[i]` | O(1) |
| first child | `i + 1` when `size[i] > 1` | O(1) |
| next sibling | `j = i + size[i]`, a sibling while `j < p + size[p]` for the parent `p` | O(1) given `p` |
| parent | in a walk, the top of the walk's stack of open ancestors; as a point lookup, the nearest `p < i` with `p + size[p] > i` | O(1) in a walk |
| position among siblings (all, named) | counted in the walk over the parent's children, only for the parents a pattern compares | O(children) |
| k-th child | `k` sibling hops from `i + 1` | O(k) |
| line and column | binary search of the document's line starts: an index over source lines, computed from the source when the document is read | O(log lines) |
| nodes of a kind | positional kind postings (`prefilter_sets`, `tape_kind_counts`) | bare index |
| nodes whose text is a term, or matches a text predicate | the term's postings (`term_postings`); for a predicate, the union of its terms' postings (`vocabulary_term`), plus the uncovered nodes (`term_coverage.missing`), which are confirmed on their text | bare index |

There is no `tid` column or array: the transducer's per-term results (`R[tid]`, 03 §16) are the
node sets above, built from the postings for the predicates the batch reads. There are no text
hashes. If point lookups of the parent prove hot, the remedy is a tape column (a parent delta, as
the spike's rec6 had), decided in the tape format, not a structure beside it.

**What a bare index is.** A key (a kind, a term, a predicate) to the set of nodes that have it:
postings at rest, a node set (`space::NodeSet`, bits over a document's nodes) in memory. It holds no
attribute of a node. Stored relations hold at most 0.25 rows per node each (the spike's limit,
Representation rule 1).

**Enforcement.**
- `scripts/check/check-node-structures.mjs` (`npm run check:node-structures`, part of `npm run
  check`): the word "sidecar" in the engine or the design documents; structs of node-attribute
  arrays; per-node records; allocations sized by a node count. Structures that exist today are
  listed in `node-structures-baseline.json` with the step that removes each; the list only shrinks.
- `content_store` test `no_relation_stores_a_row_per_node`: every table of a store holding real
  tapes has at most 0.25 rows per node.

## Consequences

- `facts::core::Sidecars` and its test were removed, and so was the `tid` vector
  (`ContentStore::term_ids`) added in step 10.
- Existing per-node structures (open item O31) are the in-memory expansions the copies brought:
  `TapeDoc`'s records, `Facts` (core, facts-a, typing), the facts vtab's projection, Metal's
  `MetalNode` upload, and the Blade (Forte) and literal sub-grammar trees. Each is removed by
  reading the tape instead; until then the linter keeps any more from being added.
- 03 §5 and §16, O7, O8 and the Phase 9 slabs row are rewritten to this decision.
