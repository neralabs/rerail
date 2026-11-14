# 0212: Kept frames are of content: digests and granules by atom strings, written mapped

Status: built (2026-11-14). Follows decision 0170 (a forked process maps its atoms to the store's) and 0197 (a
warm run writes nothing).

## Intro

The cross part keeps its relations from run to run as frames in the store (t33 K4). Each frame has a digest of its
rows and the digest that each relation it read had then. A later run takes a frame when its reads still have those
digests. Otherwise it derives the frame's relation again.

Those digests hashed the rows as this process holds them, atoms by their ids. Ids are a process's numbering, not
content. After a fork (0170), this process and the store number some strings differently, so the same rows had two
digests. The frames were also keyed by this numbering in two places: a text-owned input's documents table, and its
granules (the parts its rows are split into).

So the frames could not be written as the store numbers them. The writer refused them after a fork (`DIVERGED_TEXT`)
and dropped the refusal silently. On PostHog, 34 relations, 12 M rows and 197 MB were never stored. Every next run
found no kept part, derived every stratum again and tried to write them all again. Provisional atoms fork on most
runs, so this happened on most runs, warm or not.

## Decision

A kept frame's identity, digest, read digests and parts are functions of content, not of any process's numbering.

- **Digests read atom columns by content.** `plan::Cells` holds a relation's column types (`retain::types_of`: `s`
  for an atom column, from the domains, so kind and field columns are not atoms). A digest replaces each atom by its
  string's hash (`Atoms::content_keys`), a chunk of rows at a time under one read of the atoms. The hash is the one
  the interner already keeps for each atom, so no string is read.

  ```
  rows_digest(&Cells::new(&atoms, &types), &rows, n)      // every frame's digest
  retain::digest(source, &cells)                          // an input's
  retain::docs_of(source, g, &cells)                      // an input's digest by document
  ```

  The same function serves every kept-frame path: retain, kept strata, cone rounds, the native step's answers and
  the owner patches of kept inputs.
- **Owners that are atoms are mapped, and parted by content.** A text-owned input's documents table types its owner
  column `s` (`docs_types`). The writer maps it to the store's ids and the reader maps it back. Its granule is
  `1 + hash(string) mod GRANULES`. A document (a node's owner) stays the store's own number.
- **Frames are queued as mapped writes** (`queue_encoded_retagged`), not as `Syms::Unmapped`. Every atom a frame
  holds is in an `s` column, so after a fork its frames are written under the store's ids and read back under the
  reader's.

## Consequences

- A forked process's kept part lands in the store. The next run finds its frames current, and a warm run writes none
  of them again (0197).
- PostHog, forked cold run (169 s): the 34 relations are written. The warm run after it (8.6 s, 29,747 findings
  as before) writes no derived frame. No job is queued when it has nothing to write: frames with no frame, delta,
  retag, removal or use, or the run's atoms with none pending.
- A forked process looks up a text only on the store's side through memory (`Atoms::existing`), never with SQL
  inside the waves. A text it cannot find there is treated as a miss.
- Old frames' digests are of ids, and none matches a content digest. The first run after this change derives the
  cross part once and writes it again.
- Test: `retain::frame_tests::a_frames_digests_are_of_content_whichever_ids_its_atoms_have` uses two stores that
  number the same strings otherwise. The digests agree, the granules agree, and digests by id differ.
