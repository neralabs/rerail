# 0473. A node product is bounded by its readers' demand, or refused

Status: accepted (2026-11-18); ported from the Release-debugging branch (11a0326b9 and its uncommitted work).
Numbered 0393 on cost-classifier-metal-rf and 0461 on final-rerail-shape-rr-align (the sync branch's 0461 is "a
level's formulas as phases"), renumbered past the highest in any worktree in the sync branch's roll-up.

## Intro

Some rules pair two node ranges that no join connects. Ruff's SLF001, as written, does this: every `self`
parameter is paired with every attribute access in the corpus (`slf_class_value(m, e) :- attr_of(e, _, _),
…, text(m, "self"), …`). On 50 PostHog files that was 36 M pairs. On 1,644 files it ran past a 56 GB cap.

No join order shrinks such a rule, because its head really is that big. Replanning at run time (0203 §3a,
0206) prices it correctly and leaves it alone. Only the rules that read it can bound it: they use a few of the
pairs.

## Decision

1. **Found on the version.** A rule is a node product when its head holds two node or location columns, each
   fed by a different group of corpus-dependent literals that share no variable (`product_demand::has_products`,
   decision 0281's path). A program with no such rule decodes no tree.
2. **Bounded by its readers.** Each reader gives the product the keys its own connected context can reach: a
   guard `rr_product_demand_<rel>` leads the product's rule (`Body::demand_first`). A reader whose context is a
   product itself asks its own readers first. A reader's demand can also be projected through further readers,
   keeping order and presence witnesses (`upstream`, `nullable`; `RERAIL_READER_DEMAND=off` turns that off).
3. **Refused where nothing bounds it.** If an output, an input, or a reader with no connected keys needs every
   pair, the run stops with the error "unbounded node Cartesian product in `<rel>`: …", naming the relation and
   the reason.
4. The rewrite runs in `preparation::front` after the second prune, as a keyed step (`Pipeline::made`). It
   decodes the tree, rewrites it, and builds a new version from the result. A debug build replays it on the tree
   (stage `products`). A part prepared alone (`RERAIL_PREPARE=parts`) lacks its readers and skips it.

Ported with it:
- demand prefixes drop a read another kept read already proves (`Body::implied_read`);
- the local-join push types a column bound only by an equality (`RERAIL_PUSH_EQUAL_COLUMNS`);
- a test of the segment writer's growth, which this branch had already fixed (`slots`, e8b8125b9).

## Numbers

Profiling build, on the M3:

| Case | Before the port | After |
|---|---|---|
| SLF001 (original rule), 1,644 PostHog files | killed at a 4 GB cap | 7.9 s wall, 12.3 s user, 503 MB, 490 findings (Release-debugging also reports 490) |
| corpus0 × B-now | 463 findings, d067ec2ebf6f | the same |

## Open

- The rewrite still reads the tree. A version-native port is open (Q335).
