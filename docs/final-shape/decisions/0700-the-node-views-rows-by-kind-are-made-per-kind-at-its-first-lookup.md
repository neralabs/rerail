# 0700: The `node` view's rows by kind are made per kind, at its first lookup

Date: 2026-11-25 (the user's request: "so we do it per kind, on an as-needed basis / also it should not take 12 bytes
per entry"). Follows decisions 0353 (blocks are never copied; an index in its relation's block), 0656 (each document's
index of the codes it holds) and 0697 (the codes' atoms in the binary).

## Intro

A rule's atom `kind(n, "K")` with `n` not yet bound is a lookup of the `node` view by its kind column: the nodes of
kind K. The planner starts a rule there when K is the most selective atom; it knows each kind's count. The batch
answered these lookups from an index of the view by kind (`KindRows`, Q220), made at the batch's first lookup of any kind
of a language. It held every node of that language's documents, each as `[document, local id, tape node]` (12 bytes),
sorted by kind on the host.

PostHog's run made 534 of them, over 40.9 M nodes: 490 MB, about 467 MB live at the peak. They answered 34,934 lookups of
43 distinct kinds, whose matches are 5.7 M nodes, 14% of what was indexed. The rest, and the host's sort of it, nothing
read.

## Decision

1. **A kind's rows are made at its own first lookup in a batch.** For each language the batch has documents of, the
   batch keeps a place for each of its dialect's kinds; a lookup of a kind fills its place once. Kinds no rule looks
   up are never made.
2. **They are found by the documents' code indexes and the tape's code match, not a walk of every node.** For each
   document of the language: its index of the codes it holds (0656) skips it if it holds none of the kind's codes; a
   code few nodes have gives its nodes from the index's list; the rest are the tape's code match, 64 nodes at a time
   (`FactsIn::each_of_kind`). The matches come in preorder, document after document, so they are in row order and
   nothing is sorted. Several codes' lists are merged as they come, not sorted (`each_matching`, which `matching` now
   collects).
3. **A match is its row, 4 bytes.** The row is the document's first row plus the node's local id. A lookup's rows are
   these as they lie. A lookup kept to some documents, and one that reads columns, finds each row's document by
   advancing through the documents' first rows (`Batch::kind_matches`: the rows ascend). The tape node is not kept: a
   column that reads it selects it then (`read_matches`, `NEED_X`), and a read through the document's directory row
   reads none.

## Measured

PostHog's `len-_py` run on the M3, four runs each in turns, at loads 8 to 12, 29,965 findings each time:

| | Instructions | Peak footprint |
|---|---|---|
| Every node of a language, 12 bytes each (before) | 2,111–2,125 G | 5.01–5.13 GB |
| Each kind looked up, 4 bytes a match (this decision) | 2,105–2,116 G | 4.50–4.63 GB |

The peak falls by about 0.5 GB; the instructions are the same or fewer. System time varied from 23 to 36 s across
both engines' runs with the load, not with the engine.
