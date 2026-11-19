# t39: One document, one form (the tape behind an abstraction)

## Intro

A source document reaches the rules through up to seven forms. Each one copies the one before it, in a layout of its
own. Most are trees of pointers (`Arc<Vec<…>>`, maps keyed by strings, vectors of vectors), so none can be handed to
the GPU as one buffer, and every read of them chases pointers. The same document is decoded twice, walked node by
node into a third copy, and stored as rows twice more.

Programs went the same way and are being flattened: decision 0173 makes a program one flat image, read through
handles (`RuleRef`, `AtomRef`, …), with edits as overlays. The planner's port to those handles is t36. This item
does the same for documents. The tape becomes the one form. Every reader goes through one interface over it, and
the bytes are laid out so that the host, the SQL readers and Metal read the same bytes in place.

This is the item that follows t36's item 7. The two are independent: t36 flattens programs, this flattens documents.

## The seven forms

| # | Form | Where | What it holds | Size |
|---|---|---|---|---|
| 1 | Format 11 image | `tape::TapeImage` | the compressed tape (zstd on disk) | the tape's |
| 2 | `TapeDoc` | `tape.rs` (`pub struct TapeDoc`) | `data: Arc<Vec<Rec>>` (a record per node), a line table, points, a bitset per derived class (`labels: Vec<Vec<u64>>`), a bitset per text test (`texts: BTreeMap<String, Arc<Vec<u64>>>`) | a record a node, plus the side tables |
| 3 | Facts columns | `facts/core.rs`: `kept_tape`, `from_image` | the base relations' columns, made by a second decode of the same image | a column set a node |
| 4 | Residency's packed copy | `residency/admission/relational/syntax/packed.rs` (`Columns::build`) | parent, subtree end, kind, field, start, end, flags: seven `Vec<u32>`, built by a loop over the `TapeDoc`'s nodes | 28 B/node |
| 5 | Memo entries | `logic/plan/corpus/memo.rs` | each document's base rows | 33–42 B/node |
| 6 | Kept frames | `memo.rs` (`Frame`), the content store's admitted frames | the same rows again, with 9–13 `span_within` rows a node | as 5, and more |
| 7 | Text atoms | the facts' texts (each place found in step 1) | node texts as interned strings | 4–19 B/node |

The sizes of forms 5–7 come from the tape-format lab (experiment E9, base relations as rows against the tape, in the
`oscar` worktree's `docs/final-shape/notes/t36-tape16-lab-results.md`). The lab's earlier experiment, E7 (what readers
take), found that rules sweep a node's kind and flags. Otherwise they read only a few nodes' parts, except in tree
walks. Points, previous siblings and sizes were never read.

## What is wrong with it

- **Copies.** Forms 2 and 3 are two decodes of form 1, and form 4 is a node loop over form 2. Forms 5 and 6 store, as
  rows, what forms 1–3 already hold. The standing rule is that a tape is read in place, never decoded, with zstd
  only on disk. Every one of these copies breaks it.
- **Pointers.** `Arc<Vec<Rec>>`, the class bitsets as vectors of vectors, and the text bitsets keyed by strings are
  separate allocations. A reader chases pointers to reach them, and they cannot go to Metal as one buffer with
  offsets.
- **Size.** The row forms take tens of bytes a node, where the tape takes a few. `span_within` alone is 9–13 rows a
  node.
- **Upkeep.** Under an edit, seven forms must stay in step, each with its own freshness check.

## The target

1. **The tape image is the one form.** In memory it is the image as stored, less zstd. Nothing is decoded into
   another layout.
2. **One reader interface over it,** as a program's handles are over its image. A reader asks for a node's kind and
   flags (swept), its parent, size and span (by node id), and its text (a span of the source). It asks for its
   lines and points, and for whether it is in a class or passes a text test. Nothing above the interface knows
   the layout. The interface has a host form and a Metal form over the same bytes.
3. **Derived data is flat and keyed by the image's own ids.** Line starts, class bitsets and text-test bitsets are
   flat side columns of the document, in one allocation with offsets. They are not maps of strings or vectors of
   vectors. Where a derived column is cheap to compute on the fly, it is computed and not kept.
4. **Base relations are read, not stored.** `node`, `child`, `field`, `span_within` and the rest are views over the
   interface (the SQL virtual tables and the native steps read through it). A memo entry or a kept frame holds the
   document's tape digest and the derived rows only.
5. **Texts are spans.** A node's text is its span of the source, interned only where a rule compares it as a
   value.

## Steps

Each step leaves every caller working, and is checked against the form it replaces while it is ported, as the t36
items were: the new reading equals the old over the facts, residency, corpus, memo and session tests and dl1, and
then the old form goes.

1. **A census.** For each form: its bytes per node on dl1 and on the data-layer repository, who builds it, and who
   reads it (each reader's calls). This measures the gap and fixes the interface's calls from what readers actually
   take (the lab's E7 is the first half).
2. **The interface, over `TapeDoc` as it is.** Every reader of forms 2–4 goes through it, with no change to storage.
3. **One decode.** The facts columns (form 3) read through the interface, not a second decode of the image.
4. **No packed copy.** Residency reads the interface's flat columns, with no node loop building its `Columns`
   (form 4).
5. **No stored base rows.** Memo entries and kept frames (forms 5 and 6) stop storing base rows. Admission reads
   them off the tape, and `span_within` is a view.
6. **Texts as spans** (form 7).
7. **The tape read in place.** The interface's storage becomes the image itself, with its side columns flat beside
   it (format 16 once decision 0156 settles it). `TapeDoc`'s node records go.

Steps 2–6 do not wait on the new tape format: they move readers onto the interface, and step 7 changes only what is
under it.
