# 0244. A split part is a view of many ranges, its fields an overlay over it

Status: built (audited 2026-11-23). Decided (2026-11-15, the user's choice of t36 item 7 step 9's option 1). Continues
decision 0173 (§7) for t36 item 7, step 9.

## Intro

A corpus run splits its prepared program into a file-local part and a cross part (`file_local::assemble`, survey E1
in decision 0173 §7). Each part is a subset of the program's rules, and states fields of its own: its inputs and
outputs, and the natives, lattices and gates of its relations. The run then edits a part (E2: the cross memo, the
request cone, the run constants), and a session cuts the cross part (E3). Today each is a tree, a `clone` then
`retain`. The planner reads only versions (t36 item 7 step 8), so the parts must become versions that cost what they
change, never a copy of the program.

The builder made a view only of an edit that removes rules alone and leaves at most `MAX_RANGES` (32) ranges, and
only of a base or a view. Anything else was an overlay holding the removed set, folded when the edit reached half
its parent. A fold is a new base under the parent's ids, which copies its string table and parts: decision 0241
measured that copy at 1,300 of 1,470 samples of the maintained forms.

Measured on dl1 cold (`RERAIL_VERSION_TRACE`, each distinct split, the parts' rules matched in order against the
lowered program's):

| program | file-local part | cross part |
|---|---|---|
| 5,414 rules | 3,689 rules in 340 ranges (1,725 removed) | 1,775 rules in 346 ranges (3,639 removed) |
| 5,333 rules | 3,795 rules in 173 ranges | 1,588 rules in 180 ranges |
| 1,001 rules | 412 rules in 24 ranges | 591 rules in 23 ranges |
| the small programs | 2 to 5 ranges | 2 to 5 ranges |

No part adds a rule. The run's edits of a part remove 69 of 591, 75 of 80 and 17 of 21 of the cross part's rules, or
3 of 16 of the file-local part's, and add inputs. The session's cut removes 0 to 2 of 2,836, and adds inputs.

## Decision

A split part is made in two edits, and no new kind of version:

1. **A view of the program**: the edit removes the rules the part does not keep, and nothing else. A view is ranges
   of its parent's rules; it costs its range list, and reads its parent's rules, indexes and fields in place.
2. **An overlay of the part's own fields over that view**: its inputs, outputs, and the about rows (natives,
   lattices, gates) of its relations. It holds no rules and removes none, so it costs its fields. It is never
   folded (`Builder::finish_overlay`): a fold would copy the program's names, and an overlay that removes nothing
   reads as its parent plus its delta.

**A view's parent may be an overlay.** The run's edits of a part (E2) and the session's cut (E3) remove rules of a
part, which is an overlay over a view; they are views of it, with their own fields overlaid in turn. A view's ranges
are of its parent's rule ids; a view of a base or of a view keeps its ranges over the base image, as before.

**`MAX_RANGES` becomes a share of the parent's rules**: a view may have up to a quarter of its parent's rules as
ranges, and never fewer than 32. dl1's largest part needs 346 ranges of 5,414 rules (6%); its parent allows 1,353.
A larger fixed bound would be a number dl1 sets today and a larger program breaks: dl1 already needs ten times the
32, and a program of more rules interleaves its parts more.
A share grows with the program, so the range list stays a bounded fraction of what the view reads, and each lookup
(`rule`, `heads`, `readers`) a binary search over a list no longer than that. An edit past the share (rules kept
nearly one by one) stays an overlay of its removals.

## Why not a new kind of version

A view that holds fields of its own would be a fourth kind beside the base, the view and the overlay, with its own
image layout and every reader of the stack (`Down`, `decl`, `facts`, `inputs`, `about_facts`) taught it. The two
edits give the same reading with the kinds there are: the overlay's delta holds the fields, and its parent, the view,
holds the ranges. Decision 0173 §7 already has an overlay as "a base (or a view) and a small delta".

## Why not overlays that never fold

A part as an overlay of the program holding its removed set (1,725 and 3,639 ids above), never folded, reads its
rules by walking the program's and skipping the removed ones: every reading of the part costs the whole program,
as the `clone` and `retain` does today, and each stacked edit adds a set to skip. A view reads only its ranges. An
overlay that never folds is kept where it removes nothing (the fields' overlay above, a maintained form's,
decision 0241).

## Why not reordering the rules

Ordering the prepared program's rules by part (file-local, constants, cross) would make each part a range or two.
But a part's rules would no longer be in the program's order (the constants sit among the file-local rules), so the
keys built from the program's text would change: the choices' key, the program digests, the kept plans. Until those
are digests of a version (decision 0173 step 6), the order is part of the program's identity.

## Consequences

- `program_image::Version::View` ranges over a parent version; the builder's `view` takes any parent, and its bound is
  the share above.
- `file_local::assemble` and `cross_memo::split` make their parts as a view and an overlay of fields; the run's edits
  and the session's cut do the same over a part.
- The prepared program read from the store keeps its image: the parts are made from it.
