# t43: a kind index the architecture allows (Q167)

Surveyed 2026-11-18 at the hub (633c62a76). Sources: the ast-index-spike's design and journal (its DESIGN.md at
9449260: S053's kind index, S124's incremental upkeep, "drop absent kinds"), t39 (tape primitives, measured),
0353, 0156 E10, and 0329. Nothing here is measured anew.

## Intro

A rule often asks "the nodes of kind K" (`kind(n, #k)`). Today the answer is computed from the tape each time it
is asked. A *kind index* would keep, for each kind, where its nodes are, so the answer is read rather than
computed.

The hub had such an index (`Batch::kinds`, from Typing-support). It turned a 10-file cold run's 20 s of kind
probes into under 0.8 s per stratum. It was removed at the django-bugs merge because it was built as a `Vec` of
`node`'s rows: a copy of tape data, which 0353 refuses.

The user's view (2026-11-18): we may still need it, built in a way the architecture allows.

## What the architecture allows

- **0353 rule 3**: an index may live in the block of the relation it indexes, built from that block. Rows made
  in a separate store may not.
- **0156 E10**: an index counts as the tape's only when it is part of the tape.
- **The user, 2026-11-14**: per-kind position lists may be measured, but the design must not depend on them.
  Primitives answer from the bare tape.

So the allowed index is a section of the tape's own block (16z included). It is written when the tape is
written, read in place, optional to every reader, and never a copy of rows.

## What the spike found

- **S053, the kind index**: per-kind positions made "every node of K" a read, not a sweep. This was its largest
  single win on kind-heavy rules.
- **"Drop absent kinds"** (191 → 2 ms): a kind a file does not hold is answered without reading the file. Most
  of the win was here, not in the positions.
- **S124**: the index was kept up to date with the tape at no extra cost, because it is rebuilt with the
  file's tape.

## What the hub already has

- **Held codes**: `facts::core::matching` rejects a document that lacks the kind's code before it reads the
  tape. This is the spike's "drop absent kinds".
- **`of_tag`**: a probe reads only its own language's documents.
- **`Ctx::last`**: a repeated key is answered from the step's last probe. This covers `assign_name`'s 27 equal
  probes per chunk.

What is left is a probe of a kind that is **present but rare** in a document. That still runs `match_groups`
over the whole tape: 0.3 ns a node (t39), once per document per distinct key.

## What t39 measured of an index in the tape

| Form | Bytes per node | Effect |
|---|---|---|
| Per-kind position lists, all kinds | 2.39 | "every node of K" 10× faster; a point `has` slower; left out |
| Exact per-block code summary | 0.09–0.23 | 7× for very rare sets (0.06% of nodes); nothing for common ones |
| 64-bit filter | — | useless (every block holds most codes) |

## Perspective

The index is worth having where the sweep is paid many times over: many probes of present, rare kinds, large
batches, and the GPU, where a sweep is a launch. It is not needed where the work is the sweep anyway (common
kinds), or where held codes already answer.

Three compatible shapes, cheapest first:

1. **One sweep per step, not per key (no bytes stored).** A step with many distinct kind keys sweeps each
   document once for all of them, as the spike's batched lookups did. This is a plan shape, not an index, and
   it removes the "probed once per delta row" multiplier at its root.
2. **An exact code summary per tape block** (0.09–0.23 B/node), written with the block. It answers "does this
   block hold K" and "how many K", so the planner gets exact counts without reading nodes, and sweeps skip the
   blocks without K. t39 measured it as cheap, and it helps exactly the rare-kind case left over.
3. **Kind postings as an optional tape section**, positions per kind inside the tape's own block, perhaps only
   for kinds under a rarity threshold, so the 2.39 B/node of all kinds is not paid. Readers use it when present
   and sweep when not, so the design does not depend on it.

Never: node rows gathered by kind into a `Vec` or table outside the tape (`Batch::kinds`'s form).

**Recommendation**: measure Q167's 10-file cold run at the hub first. The held codes and `Ctx::last` may already
cover most of the 20 s. If they leave it slow, build (1), then (2). Take (3) only if a measured workload still
spends its time in rare-kind sweeps after those.
