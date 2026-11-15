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
6. **The split's fixpoints in numbers.** `split_in` numbers each rule in scope once (`file_local::shape`):
   - variables by first appearance, each with its class under the rule's equalities (a closure under them is the
     classes it touches);
   - value tapes and binders after the variables;
   - derived relations by number, with their location columns a vector by number.

   Each round's location check and file-local check then works on bits and a union-find over a rule's ids, not on
   `BTreeSet<String>` closed by scanning again, relations looked up by name, and a `format!` per tape. The check by
   text still runs, once, for a rule the numbered check fails, to give its cause. Debug builds assert that the two
   checks agree on every rule.
7. **The strata on numbers.** `front::strata`'s graph is built on numbers (`Graph`):
   - the derived relations in their order, each one's dependencies sorted as `Edges` sorts them;
   - the search (`tarjan_ids`), `together`'s rings and the stratification check run on it.

   The edges by name, a sorted map of string pairs with a `to_string` per body atom, are made only for a caller that
   reads them. The callers that read only the components (about twenty) call `front::components_of`. The public
   `tarjan` numbers its nodes too. The components, their order and the errors are unchanged. The merged
   Typing-support-final-shape work (t36 item 6) already gives `keeps` and the negation split strata by `RelId`;
   this is the same cut for the passes that still hold a tree.
8. **Domains, text-kind demand and keep factors on numbers.**
   - `lower::domains` numbers the relations in declaration order. Each relation's column domains are a vector, and
     the rules and readers are listed by number. `rule_vars` takes a lookup by atom, and `columns_of` gives the
     by-name view to the callers that read one.
   - `text_kind_demand` (`facts/vtab/syntax.rs`) keeps its kind table by number and folds its fixpoint
     (intersections and unions) on it, with the heads found once.
   - `keeps::factor_parts` works on numbered variables and bit sets. Only the answer is mapped back to names.
9. **The planner's column statistics by relation.** `Model::ndv`, `Stats::fan` and `Stats::dfreq` were maps keyed
   by `(relation, column)`. A strand's model (`Model::fork_with`) copied each column's entry with its own copy of
   the relation's name, and each frequency map whole. They are now a `ColMap`: per relation, one shared vector of
   its columns. A fork takes a relation's vector by a count, and a write copies only that relation's vector.
   `absorb_with` sets the columns below the width from the fork's vector, as moving each entry did. Reads take
   `at(relation, column)` and build no key. Keying `Model` by `RelId` as a whole (0173 stage 2) waits on the
   planner taking a program version (t36 item 7), which is not started.

   *Merged (2026-11-14):* the hub had meanwhile numbered the statistics (decision 0209 step 1: `table::ColMap`,
   each relation's columns a block of slots in one array, by `RelId`), so that form is kept and this `ColMap` was
   dropped. What it saved on a fork is kept on 0209's form: `Stats::dfreq` holds each frequency map in an `Arc`,
   so a fork copies a pointer per column, not the map.
10. **The split numbered for the run.** §6 numbered each rule in scope again at every split, and the constants
    and filters fixpoints still read names. Now:
    - A rule body's form (`shape::Form`: its variables, their classes, the value tapes and binders, each read's
      relation by its name's address) reads only the body. So the run keeps it with the body's digest in the split
      memo's bodies (`split_memo::Body`), made the first time a split asks for it. A split numbers only the bodies a
      round rewrote.
    - Each split numbers its relations by their name's address (`shape::Rels`). A `Name` is interned, one address
      per name, so a read's relation is found with no hashing or comparing of its text. What the split asks of a
      relation by name (a fact, an input, a run constant, a chunk's value, a text test, its text columns, its base
      location columns, its rules, its readers) is asked once per split.
    - The constants, locations, filters and file-local fixpoints all run on these numbers, and their rounds are lists
      of numbers.
    - The keys read each rule's body entry once. Each thread of the pool takes the memo's lock once for its share,
      not once per rule. The relations' digests are keyed by `Name`.

    The split's results are unchanged. A dump of every split and every `constant_relations` answer on dl1 (48 and
    52) was the same, byte for byte, from the old code and the new.
11. **`simplify`'s congruence on numbers.** A run calls `congruence` about 24 times (each `simplify`, each
    `merge_identical` after the file-local joins, `corpus::scoping`), over 27,664 rules in all on dl1. Only 30% of
    those are a body an earlier call saw (19,306 distinct), so keeping the templates for the run would save little
    and be a store of its own. Instead each call does less:
    - the candidates are numbered once, and every other relation a rule names after them;
    - a rule's template keeps the relations it reads as numbers, not a `String` copy of each name;
    - each refinement round labels a read by a vector index (its class, or its own number), not by a lookup in a
      string-keyed map and a `&str` label compared by text;
    - what keeps a relation its own (`fixed`: inputs, demand, gates) is a set made once, not a scan of each list
      per candidate;
    - a candidate's distinct rules are counted with foldhash, not SipHash, and its column types are `Name`s;
    - a rule is copied only when its constants are abstracted.

    Every `congruence` result on dl1 (26) was the same from the old code and the new.
12. **The lowerer's span patterns on numbers.** `span_patterns` keyed each column's spans by `(String, column)`,
    with a `to_string` for each lookup, and its rules, readers and reading sites by text. The relations are now
    numbered by their names' addresses, and the spans, the demand, the readers and the sites are vectors by
    number. The rounds are unchanged: each reads the columns as the round before left them. Every pass's result on
    dl1 (99 lowerings: `rx_spans`, the AST patterns) was the same from the old code and the new. dl1 has no span
    pattern, so the lowerer's span tests are what exercises the fixpoint.

## Measured

dl1, `RERAIL_THREADS=4`, cold, `--profile profiling`, alternating A/B runs:

| Step | Instructions (medians) |
|---|---|
| hashing | 24.0–26.0 → 22.8–24.6 G (ranges) |
| compiled spans | 22.95 → 22.65 G |
| incremental split keys | 23.78 → 23.47 G; keys 4.6% → 0.6% of the main thread |
| `canonical` | 23.3 → 22.78 G |
| sharded names | ~22.95 → ~22.83 G; `Name::new` 872 of 13,605 samples → 2 of 13,269 |
| numbered split | 23.08 → 22.47 G (load 52–65) |
| strata on numbers (after the Typing merge) | 22.60 → 22.25 G (load 20–24) |
| domains, text-kind demand, keep factors | 22.64 → 21.96 G (load 63–78) |
| column statistics by relation | 21.71–21.78 → 21.49–21.65 G (load 26–28) |
| the split numbered for the run | 21.61 → 21.24 G (medians of five pairs, load 78–98) |
| congruence and span patterns on numbers (together) | 21.37 → 20.80 G (medians of five pairs, load 57–69) |

The set as a whole, against the engine built just after the hub merge (one of the earlier steps was already in
that engine), at load 76–84: instructions 23.64 → 22.90 G, CPU 2.99 → 2.83 s (medians). Walls at that load are
noise.

## Found along the way

The session tests `a_session_derives_again_*` hung: the test held `switches::set`, and the test stores'
template, built on its own thread, ran phase 0's bootstrap run (decision 0197), which waited in
`switches::reading` for the test. The test thread joins the template thread at once, so the template thread now
runs at its starter's depth (`switches::within`) and takes nothing.

## What remains

This line of work ends at §12: decision 0225 (draft) replaces constant-factor work on the tree's passes with their
move onto the program's image, as rules the engine maintains. The numberings above are stand-ins until then.

These are open items (O-split-in-strings and the others in open-items.md):
- A split still starts each relation outside its cone from the kept split's string sets, and gives its results as
  string sets and maps (`Split`), which `split_explained` copies. `RuleIndex` is keyed by text.
- `canonical` still builds a relabelled copy of each rule and prints it, once per rule per `congruence` call.
- `keeps::choose` is on names until 0173 stage 2.
- `Board::finish` is 15% of the main thread, mostly waiting on the pool (`LockLatch`).
- `memcmp` is 14.6%, spread over the string-keyed BTree lookups of many passes.
- `keeps::choose` still keys its refusals, bounds and estimates by name (`BTreeMap<&str, …>`, `BTreeSet<String>`),
  and `Model`'s other maps (cards, memo, delta rows) are by name. Both wait on 0173 stage 2 (`RelId` keys).
