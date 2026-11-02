# 0026: nodes live in tapes

Status: accepted (2026-11-02, user decision; scope narrowed the same day to what is stored)

## Context

The plan kept per-node arrays beside the tape under the tape's digest and called them sidecars:
parent, position among siblings (all and named), line starts, the term id (`tid`) and text hashes
(03 §5 and §16, O7, the Phase 9 slabs row). The user's ruling: nodes are stored in tapes, and the only
other stored thing over nodes that may be considered is a bare index: a key to the set of nodes
that have it. The rule is about the database: "we only need to remove those that are stored
on-disk". In-memory structures a consumer builds from a tape while it works are not covered,
as in the spike, whose rule is "nothing may *store* one row per node" and whose dead end D14 was
*persisted* sidecars.

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

The tape (v9np, decision 0025) is the only per-node structure in the database. Everything else
stored over nodes is a bare index, or an aggregate, at most 0.25 rows per node per relation (the
spike's limit). A reader needing a node attribute reads it from the tape's columns, computes it
in its walk, or reads a bare index, as the table below says; what it builds in memory meanwhile
is its own working state, not stored, and not ruled out. The word "sidecar" (a structure kept
beside the tape under its digest) is not used: a stored one is ruled out, and an in-memory one is
just the reader's state.

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
| parent | in a walk, the top of the walk's stack of open ancestors; as a point lookup, by descent from the root (`prefilter::tapes`); see "Parent lookups" | O(1) in a walk |
| position among siblings (all, named) | counted in the walk over the parent's children, only for the parents a pattern compares | O(children) |
| k-th child | `k` sibling hops from `i + 1` | O(k) |
| line and column | binary search of the document's line starts: an index over source lines, computed from the source when the document is read | O(log lines) |
| nodes of a kind | positional kind postings (`prefilter_sets`, `tape_kind_counts`) | bare index |
| nodes whose text is a term, or matches a text predicate | the term's postings (`term_postings`); for a predicate, the union of its terms' postings (`vocabulary_term`), plus the uncovered nodes (`term_coverage.missing`), which are confirmed on their text | bare index |

Nothing stores a term id per node: the transducer's per-term results (`R[tid]`, 03 §16) are the
node sets above, built from the postings for the predicates the batch reads. No text hashes are
stored.

**Parent lookups, as ast-index-spike does them** (`notes/t4-ast-index-spike.md` §2.1, §2.5, §3,
§8.5):
- The shipped record, rec6, is `[kf:2][par:2][size:2]`: `par` is a **parent delta stored in the
  tape**, and the parent is `pre − par`, one subtraction (D S047 L2983-3005; HI L240). The delta
  fits in 1 B 96.6% of the time and in 2 B 99.69%; larger ones spill to an overflow list at about
  0.05 B/node (D S039 L2652-2727). The spike's indexes also say "parent, cpos, hi and depth are
  recovered, not stored" (D S012); §2.1 notes the two statements are not reconciled, and the
  shipped format is the delta.
- A climb is repeated subtraction: hop to `pre − par`, check the hop's kind (S207: 147 → 26 ms),
  carry the root's kind and size from the last hop (655 → 137 ms). The last child of `i` is found
  by climbing parent deltas from `i + size − 1` (S180, S276).
- Climb versus descent is a planning choice: anchor on the rarest kind, climb toward the root,
  then descend into required children, size-bounded and pinned to the spine the climb came up
  (S217). Forcing the climb everywhere was rejected (SQLite > 215 s vs 38.6 s); the SQLite rule
  "climb iff leaf_freq ≤ 10 × anchor_freq" was fitted with sidecars and is to be replaced by a
  cost expression (ledger #13, open).
- A descent from the root is the spike's fallback for a missing parent, and was the slow side:
  a top-down recursive walk took 33.5 s against the leaf climb (D36).

**Rerail's tape differs here, and keeps no parent column** (decided 2026-11-02 after list step
12, decision 0025 amended). The spike stored the delta because its SQL read the tape blob in
place (`substr` on rec6), so a climb had nothing to compute parents from. Rerail's tapes are zstd
frames, decoded whole before any read; one stack pass over the sizes at decode gives every
parent, so a climb is the spike's O(1) arithmetic without the stored bytes. `TapeDoc` does this at
restore, and the SQL fallback's reader (`prefilter/tapes.rs`) now does too. Measured on the lab's
captures: the fallback's execute CPU is 1.55–1.63 s with parents rebuilt at decode and 3.53–3.63 s
by descent (same VM steps), the spike's D36 again (descent the slow side). Revisit if a reader
ever reads tapes in place without decoding them (the GPU slabs, list step 28, upload decoded
columns, so they rebuild parents in the same pass).

**What a bare index is.** A key (a kind, a term, a predicate) to the set of nodes that have it:
postings at rest, a node set (`space::NodeSet`, bits over a document's nodes) in memory. It holds no
attribute of a node. Stored relations hold at most 0.25 rows per node each (the spike's limit,
Representation rule 1).

**Enforcement.**
- `content_store` test `no_relation_stores_a_row_per_node`: every table of a store holding real
  tapes has at most 0.25 rows per node. S7 asserts the same over its store, which holds every
  family the slice writes (derivations, matches, findings, statistics).
- `scripts/check/check-node-structures.mjs` (part of `npm run check`): the word "sidecar" in the
  engine or the design documents.

## Consequences

- `facts::core::Sidecars` and its test were removed, and so was the `tid` vector
  (`ContentStore::term_ids`) added in step 10.
- No stored relation is per node. S7's store (322,472 nodes): the largest are
  `derivation_inputs` 0.12 rows/node, `term_postings` 0.090, `atoms` 0.044, `term_node_freq`
  0.041, `term_freq` 0.038, `term_coverage` and `tape_kind_counts` 0.035, `vocabulary_term`
  0.024, `slice_match` 0.016.
- The in-memory per-node structures the copies brought (`TapeDoc`'s records, `Facts`, the facts
  vtab's projection, `MetalNode`, Forte's and the literal sub-grammars' trees) are readers'
  working state, never stored, and stay (O31 closed).
- 03 §5 and §16, O7, O8 and the Phase 9 slabs row are rewritten to this decision.
