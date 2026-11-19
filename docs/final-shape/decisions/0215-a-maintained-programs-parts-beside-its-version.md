# 0215. A maintained program's parts are a typed form beside its version

Status: decided (2026-11-14). Continues decision 0173 (§7) for t36 item 7, step 1.

## Intro

A session's maintained program (decisions 0077 and 0090) has three parts no other program has:
- `again`: for each stratum that may be derived again, its relations and their pieces, the deltas it reads, and its
  rules for deriving it again: its rules as given, then its deltas against its old rows. These rules are not the
  program's rules. The program's rules maintain the stratum from deltas, and these replace them when it is
  derived again instead.
- `together`: the sets of relations run as one component;
- `shapes`: the directives that narrow a form rule's search.

A program image (decision 0173) had no form for them. `Builder::of_program` refused a program that had them, so
`plan::run` read a maintained program from the tree. The planner must read every program it runs from a version
(t36 item 7).

## Decision

The parts are one typed form, `body::Maint`, which the program's version (`body::Placed`) holds beside it.
- `again`'s rules and deltas are rules of an **overlay** of the program's version. The form holds each stratum's
  rule ids there, so they are rules a reader reads by handles. They are not the program's rules, so no reader of the
  program sees them unless it asks.
- The strata's relation sets (its own, the relations it owns, the deltas it reads, its old rows), and the
  `together` sets, are relation ids of that overlay.
- A shape's head, literal, wait and text are atoms of the overlay.

These parts are not in the image, as about rows or otherwise:
- **Not the program's rules.** Rules held in the image would be read as the program's by every reader of its rules
  (strata, heads, readers). Each such reader would have to skip them.
- **Variants are overlays.** Decision 0173 §7 makes every variant of a program an overlay of it, and the
  alternative rules are one. The run already held its maintained forms' rules this way (t36 item 6, step 6).
- **Never stored.** A maintained program is made in its session and is never stored, and the `.rr` text has no
  form for these parts either. So no stored format has to carry them, and no image decodes into a program
  without them.

So `Placed::of_program` makes the whole image of any program: the image of its other parts, and `maint` where it
has maintenance's parts. `Builder::of_program` still refuses them, for the store's callers, which need an image that
decodes back to the program.

The passes that rewrite a program before its run (`keeps::choose`, `demand::split_negations`) give nothing for a
maintained program, as before.
