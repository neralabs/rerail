# 0210. A batch's preparation hashes, compiles and locks less

Status: built (2026-11-14). Follows decision 0197; dl1's cold batch, main thread.

## Intro

A cold batch of data-layer policies (dl1) spends most of its main thread preparing programs: simplifying them,
splitting each into its file-local and cross-file parts, and planning. A profile of that thread, after the hub
merge, showed work that repeated without need:
- the default SipHash hasher in the maps of simplify, lowering and the rule index;
- every regex of a projection's spans compiled again for each batch;
- the split's keys hashing every rule of the program again at each split, though a round changes few rules;
- `simplify::canonical` building a map, a `format!` and an interned name for every variable of every rule;
- one process-wide lock in front of the interned names, which the pool's threads waited on.

Each fix below changes how much work is done, not what comes out. dl1's findings are identical after each one,
and the split and magic self-checks pass.

## Decision

1. **Hashing.** simplify, `plan::lower` and `RuleIndex` use FxHash, and the split memo's keys a fixed-seed
   foldhash. A run's model analysis is dropped behind the run (`threads::drop_behind`), not on its main thread.
2. **Compiled spans.** A projection compiles its span regexes once (`syntax::CompiledSpans`), and each batch
   built from it reuses them.
3. **Incremental split keys.** The run's kept splits also hold the digest of each rule body they have hashed,
   keyed by the body list's address. The entry holds the list itself, so the address cannot be reused, and an edit
   copies the list before changing it. The next split hashes only the bodies a round rewrote.
4. **`canonical`.** It numbers variables by scanning the few names seen so far. The names it writes (`v0`, `v1`,
   ..., `@`, the placeholder) are made once by its caller (`CanonicalNames`) and passed in. The canonical text is
   unchanged.
5. **Sharded names.** `front::Name`'s table is 64 shards, picked by the text's FxHash. One text always lands in
   the same shard, so a name is still one allocation and equality by address still holds. Each shard sweeps its
   own dead entries.

## Measured

dl1, `RERAIL_THREADS=4`, cold, `--profile profiling`, alternating A/B runs:

| Step | Instructions (medians) |
|---|---|
| hashing | 24.0–26.0 → 22.8–24.6 G (ranges) |
| compiled spans | 22.95 → 22.65 G |
| incremental split keys | 23.78 → 23.47 G; keys 4.6% → 0.6% of the main thread |
| `canonical` | 23.3 → 22.78 G |
| sharded names | ~22.95 → ~22.83 G; `Name::new` 872 of 13,605 samples → 2 of 13,269 |

The set as a whole, against the engine built just after the hub merge (one of the earlier steps was already in
that engine), at load 76–84: instructions 23.64 → 22.90 G, CPU 2.99 → 2.83 s (medians). Walls at that load are
noise.

## Found along the way

The session tests `a_session_derives_again_*` hung: the test held `switches::set`, and the test stores'
template, built on its own thread, ran phase 0's bootstrap run (decision 0197), which waited in
`switches::reading` for the test. The test thread joins the template thread at once, so the template thread now
runs at its starter's depth (`switches::within`) and takes nothing.

## What remains

These are open items (O-split-in-strings and the others in open-items.md):
- `split_in` is about 10% of the main thread, with string-keyed sets and a `format!` per tape.
- `Board::finish` is 15% of the main thread, mostly waiting on the pool (`LockLatch`).
- `memcmp` is 14.6%, spread over the string-keyed BTree lookups of many passes.
