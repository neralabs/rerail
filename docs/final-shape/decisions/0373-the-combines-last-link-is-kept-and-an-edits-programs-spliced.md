# 0373: The combine's last link is kept, and an edit's programs are spliced into it

Status: built (2026-11-17), measured on ep2 below. Work queue Q18. The first step of 0370 (0345's stage 3): the
link before the combine's tail, kept as 0368 keeps the parts' links.

## Intro

A run of many programs combines them into one program before preparing it. Each program is lowered in its
libraries' scope (its image is kept, 0246 §6), its header is decoded, and the merge reads it: what it takes from the
library (its cone and request, 0089 §3), its kind sets, its globals and scope (its policy, 0091), and its relations'
names. Then every program's rules are copied into the combined program under their new names.

On an edit of one policy, 316 of the 317 programs are the same as last time, and so is their part of the combined
program. Yet each edit decoded all 308 headers (0.10–0.14 s), merged them (0.027 s), and copied all their rules.

## Decision

1. **The combine's link is kept by the run's shape** (`kept_link::family`): the engine, the number of programs, the
   rows the run adds (0249), the library's images, and the switches the combine reads (`RERAIL_REQUESTS`,
   `RERAIL_LIBRARIES`). An edit of a program's text leaves the shape as it was.
2. **The link records a slot for each program**: its image's key (its text and scope, `Scoped::program_image`) and
   its interface. It also records how many of the linked rules are each part's (the header's, the library's, then
   each program's, after `core.rr`'s repeats are dropped). Beside those are the run's requests by key (each one's
   number and the program that asked it first), the kind sets by their kinds, the bound, and the tail's hiding and
   policies.
3. **A program's interface** is everything the merge reads of it besides its rules' bodies, in the merge's order:
   its index, its bound, whether it asks for guests, its request (key, number, whether it asked first), its policy,
   its text globs, its kind sets and their shared names, its own relations, its header as the merge links it, and the
   rules it holds of `core.rr`'s relations. Two programs with the same interface in the same slot merge alike: the
   same header, the same requests and copies, the same kind sets, the same policy and the same dropped repeats.
4. **An edit splices.** Only the programs whose image key differs from their slot's are lowered. Each one is read as
   the merge reads it (`own_part`, the request, `undo_on_request`, the kind sets, the globals, the policy). Its
   request must be one the record holds, asked first by it or by an earlier program, and its kind sets must be ones
   the record names. If its interface is its slot's, the last link is opened (one image) and a new version derived
   under its ids: each changed program's rules written in its slot's place, renamed as the merge names them, and
   every other part's copied as they are. The tail is given the record's hiding and policies. The new link is kept.
5. **Otherwise the programs are linked whole**, as before, and that link is kept. A changed interface, a request or
   kind set the record lacks, another bound, or counts that do not match the image all relink.

## What it does not do

- The tail still runs on the whole combined program (fact spaces, hiding, the sources on request, the merges): 0370's
  next step.
- Each text is still read and each scope seeded before the splice (0.043 s and 0.009 s), since the image keys need
  both.
- The kept link is kept in the image store, as the programs' images are, and its lease is renewed where an edit
  reads it (0250). One that collection reclaims makes the next edit link whole and keep it again.

## Checked

`RERAIL_COMBINE_LINK=check` splices, links the programs whole beside it, and asserts that the two are one program
(size and digest) with one tail (hiding and policies). On ep2's edit (e49) they were. `RERAIL_COMBINE_LINK=whole`
links whole every run. A debug build's own check (the trees linked beside the versions, `combine_tail`) cannot
reach the splice on ep2's rules yet: the lowering's debug checks fail first (work queue Q90).

## Measured

ep2 on the m3 (300 files, 317 rules, `RERAIL_PREPARE=parts`, an edit of B101's body, profiled). e45 was the cold run;
e46 and e47 were edits, and each spliced 1 of 317 programs. Per edit:

- the splice: 0.015 s, of which keeping the new link was 0.007 s and reading the last one 0.005 s;
- gone: the 308 header decodes (0.10–0.14 s) and the merge (0.027 s);
- `combine`: 0.50 s → 0.46 s;
- the edit: 2.27 s → 2.20 s CPU (28.4 G → 28.1 G instructions, e50);
- rows: 1,334, none differing from h1.
