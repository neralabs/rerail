# 0252: the constants' and the cone's rounds' records go with their plans

Status: built (2026-11-15). Amends the observer's collection (`observe::collect`), decision 0145 (the cone's
rounds) and the run's constants (`corpus::constants`).

## Intro

The observer's store keeps records of what runs kept, under a prefix each: a whole-run cutoff's (`cutoff:`), the
run's constants (`constants:`), and the cone's rounds (`cone-rounds:`). Collection dropped a cutoff's record once
none of the plans it names was live. The other two named no plan, so collection kept them for ever. Each build of
the engine, or edit of a program, keys them anew, so the old ones piled up.

## Decision

A record that names the plans it is kept for goes when none of them is live, as a cutoff's did:

| Record | Names |
|---|---|
| `cutoff:` | the plans the run used (as before) |
| `constants:` | the plans the constants' run used, in its consulted lines; filtered out before its reads are checked (key v3) |
| `cone-rounds:` | the part key its frames are kept under (`part_keys`), a fourth line its reader does not read |

A plan is live while a kept manifest names it: the outer run's manifest names the constants' plans (0250) and the
rounds' part key (0227). A `constants:` or `cone-rounds:` record naming no plan is from before this, and goes.

## Tests

- `collection_drops_what_no_run_asks_any_more` (store): the live constants' and rounds' records are kept; the dead
  ones, and one naming no plan, go.
