# 0368: The last link of a program's parts is kept, and an edit's parts are spliced into it

Status: built (2026-11-17), measured on ep2 below. Work queue Q18 (where an edit's 2.65 s goes). Builds on 0345
(a part is prepared alone, against its interface) and its two stages, and on 0103 (what is kept is a stored fact,
under the key of what it was made from).

## Intro

0345 made an edit of one policy prepare one part. But each stage still read every other part's kept image and
linked all of them again: 317 image reads and a full link in each stage, about 0.5 s of an edit's 0.87 s in
`prepare`. The link's output for the 317 parts that did not change is the same as last time.

So the link is kept too. A compiler that rebuilds one object file relinks the program; an incremental linker
patches the last binary instead, where the changed object's symbols are the ones the binary already has. This is
that, for the parts.

## Decision

1. **The last link is kept by the program's shape, not its content.** Its key (`link_key`) is the stage, the
   engine, the header every part shares (0345's `common_of` with the shared relations), the program's outputs, the
   stage's settings (the dialect before lowering; whether exports are factorized and the push mode after it) and the
   parts' names in order. An edit of a part's rules leaves it the same, so the edit finds the last link.
2. **A link records its slots.** For each part, in order: its key (0345 §5), its interface's digest, how many of
   the linked rules are its, and the sources it was read from. Before lowering the link also records each part's
   renames and what the passes found (as a part's entry holds them).
3. **A part's interface** is what the link reads of it besides its rules' bodies. Before lowering it is the part's
   header before and after its passes, what it makes and reads, the relations its rules derive, and what its passes
   found (tests, regexes, matchers, demand, reaches). After lowering it is its headers, what it makes and reads, and
   what its rules derive. Two parts of one interface link alike: the same renames, header changes and numbering of
   the parts after them.
4. **An edit splices.** Every part's key is computed, as before, from the program alone. Only the parts whose key
   differs from their slot's are read or prepared. If each of their interfaces is its slot's, the last linked
   version is opened (one image), and a new base is derived under its ids: each unchanged slot's rules copied as
   they are, each changed part's rules written in its place, renamed as the last link renamed them. What the link
   found is the kept one. The other parts are not read, and nothing is linked again.
5. **Otherwise the parts are linked again**, as 0345 links them, and the new link is kept. A changed interface, a
   part added or removed, a missing image, or a slot count that does not match the image all relink.
6. **The run names what it used.** A splice names the link's sources, the changed parts' sources, and each
   unchanged slot's recorded sources, so collection keeps the same parts live as a full link's run did.

## What it does not do

The passes over the whole that follow each stage (lowering, the split, the merges of identical relations, the
bound check) still run on the whole linked program. They are 0345's "what stays global", and the combine's tail
before stage 1 is 0345's stage 3.

## Measured

To fill in from ep2 (e24 cold, e25 and e26 edits of B101's body, `RERAIL_PREPARE=parts`, the m3).
