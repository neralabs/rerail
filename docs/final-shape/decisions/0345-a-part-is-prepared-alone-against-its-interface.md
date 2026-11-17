# 0345: A part is prepared alone, against its interface, and the parts are linked

Status: proposed (2026-11-17). The second answer to work queue Q18, beside 0333 (Typing-support's step P), to
compare on ep2. Builds on 0089 (the parts of a combined program: the library, each program's `p<i>_`, each request's
`q<k>_`), 0225 §7 (a prepared program per module) and 0103 (caches are two rules).

## Intro

An edit of one policy should cost that policy. Today an edit of B101's body prepares the whole combined program
again: about 10,000 rules through the passes before lowering (prune, simplify, demand, the text passes) and after
it (push, factorize, cut, locate, merge).

0333 answers this with deltas. It stores each pipeline's last run, and a new process reruns the passes on the edit.
Its cost is the stored run (written each edit, opened each edit), and the passes 0333 §6 leaves whole.

This decision answers it with separate compilation, as a compiler builds one module at a time:

- **A part is prepared as a program of its own.** Its rules, with the rest of the program as its environment: what
  it reads from other parts are its inputs, and what other parts read of it are its outputs.
- **A prepared part is kept by what it was made from.** Its key is its own rules and its interface. An edit of one
  policy misses that policy's part only, unless it changes what the policy reads of the library.
- **The parts are linked.** The prepared parts are put together, and only what crosses parts runs over the whole.

Nothing is stored about a run, and no pass needs a delta form. A pass written for a whole program runs on a part
unchanged: it already leaves inputs and outputs as they are (`kept`).

## Decision

1. **The parts.** After the combine, each rule belongs to a part:
   - program `i` if it reads or makes a relation named `p<i>_`;
   - request `k` if it reads or makes a relation named `q<k>_`;
   - else the library part.

   This holds for the rules the combine makes over all programs: a request rule of an attribute
   (`rr_value_req(n) :- <a reader's literals>`) belongs to the program whose rule it was made from. A relation made
   in several parts (`rr_value_req`) is an output of each part that makes it and an input of each that reads it.
2. **A part's interface.** Its inputs are the relations it reads that another part makes, with their declarations.
   Its outputs are its relations that another part reads, and the program's outputs it makes. Within a part the
   passes treat these as they treat any input and output: never pruned, never projected, never renamed.
3. **A part is prepared by the same passes.** `preparation::front` runs on the part's program unchanged.
4. **Names a part makes are its own.** A pass that makes a relation numbers it within the program it runs on, and
   readers parse those names (`strtest_<k>` is test k, `matchall_<k>` regex k, `rr_ast_<k>_<m>` has m captures).
   So a name keeps its form at the link: the vocabulary's tests and regexes are renumbered past the earlier parts';
   a counter-numbered family (`rr_ast_<k>`, `rr_ast_d<n>`, `rr_long_text<n>`, `rr_node_text<n>`) keeps its number
   unless the program or an earlier part took it, and then takes the family's next free one; a name with a part's
   tag is that part's already; any other name two parts made gets the part's suffix (`_x<j>`). Every name of a
   renamed relation in what the passes found (the matchers, the vocabulary) is renamed with it.
   A part's inputs are what another part makes; a base relation, or a request the runner answers (`dir_entry`,
   `loaded`), stays as the whole program has it. A part's header names its own relations, what it reads, the base
   relations and what natives read, never another part's.
5. **The key.** A prepared part is kept under a digest of its header (its own relations' fields, and the fields
   every part shares), its rules' hashes in order, the engine build, and the switches its passes read: made without
   building the part. It is kept as 0103 keeps a cache: a key rule
   and an output rule over the part's image. A missing part is prepared; a part found is read as it is.
6. **The link.** The parts' rules and headers are put together, with the combined program's inputs and outputs.
   What crosses parts then runs over the whole: merging the relations that are one, and the passes after
   lowering (lowering, push, factorize, cut, locate). Making those cost their parts is the next stage.
7. **Checked against the whole.** The findings are the whole program's (ep2's rows against h1), and in a debug
   build every part's prepared form is the front's on that part.

## What it costs

- **Optimizations across parts are lost.** A library relation a program reads is an output of the library part,
  so it is never projected to the columns that program reads, nor demand-pushed into it. ep2 measures whether the
  run is slower for it.
- **A cold run prepares every part once.** That is about the same work as preparing the whole, plus one key per
  part.

## Stages

1. **The front per part** (this decision's first build): the parts, their interfaces, the front on each part, the
   renaming, the key and the link before lowering. Measured on ep2 against 0333 (P) and against the whole.
2. **The passes after lowering per part.** Push, factorize, cut and locate read two analyses: the fields (a part
   has them from its own declarations) and the file-local split, which is global. A part's split is decided at its
   interface:
   - **The split is computed once over the linked, lowered program** (0.02 s on ep2), and cut into parts again.
   - **A part carries its inputs' classification.** Each input another part makes is qualified, in the part's
     header, with what the split found of it: file-local or not, a constant, a filter, its location and
     same-document columns. The part's split treats a qualified input as a derived relation out of its scope, read
     from those qualifiers as a scoped split reads an earlier one (`split(scope, prev)`); its own relations are its
     scope. Its classification is part of its key.
   - **A part's outputs name their readers elsewhere.** An output only another part reads is qualified
     `part:out` (kept, never located for being an output); one a cross-file rule elsewhere reads, `part:cross-read`
     (an export here); one a file-local rule elsewhere reads, `part:local-read` (never located).
   - **The push's mode is the run's.** A part's cross-file part reading a base relation after the first push puts
     the whole run in the second mode. Each part keeps its share as a fact, read before its image; a part that
     pushed in the other mode is prepared again in the run's, under a key naming it (`part:push-decided`).
   - **The export cut and the factorizing are per part, and conservative across parts.** An export another part
     reads is a part output, and outputs are neither cut nor factorized: these are lost across parts, as projection
     across parts is lost before lowering.
   - **What stays global:** lowering (0.03 s), the merges of identical relations and repeated rules (helpers two
     parts pushed alike are one relation, named by content), and the split into file-local and cross-file
     programs. These are linear passes over the linked program; the keyed work is the rewriting.
3. **The combine per part.** Hiding's rounds and the fact spaces over each program's part.

## How it is compared with 0333

Both are measured on the same ep2 seed and edit (B101's body), in a new process, on the profiling build, on the
same host. The measures are `combine` and `prepare` instructions, bytes written per edit, the cold run's cost, and
the rows (0 differing from h1). `RERAIL_PREPARE=parts` selects this decision; the default stays the whole program
with P until the comparison is made.

## Measured (stage 1, 2026-11-17)

ep2 on the m3, beside 0333 (work queue Q18 has the table): the rows are h1's; phase 0's and the sources phase's
prepared programs are the whole's exactly. The cold run is 942 G against 980 G. An edit is 50.8 G against 0333's
40.7 G, and writes 2.3 MB against 12 MB. One part of 318 is prepared on the edit. What is left over 0333 is whole-
program work done on every edit: building each part to key it (0.55 s), reading the parts (0.23 s) and the link
(0.79 s). The next steps are a part's key from its rules' digests, and a kept link that relinks the missed part.

Then keyed without building (same day): a part's key is its header and its rules' hashes, so only a missed part is
built. An edit is 43.9 G against 0333's 40.7 G; what is left over it is the link (0.74 s CPU) and reading the 317
parts (0.27 s).

Then a kept part holds its own header only (same day): the fields every part shares are the link's, and a part
keeps only those its passes changed. The link was quadratic in them, not in the rules. An edit is 35.5 G against
0333's 40.7 G; before lowering is about 0.4 s CPU, and the passes after lowering (stage 2) are now the larger cost.

Stage 2 built (same day): rows h1's; an edit is 36.3 G (stage 1 35.5 G, 0333 40.7 G), and no part's passes after
lowering run on it. The cold run is 1,169 G against the whole's 980 G.
