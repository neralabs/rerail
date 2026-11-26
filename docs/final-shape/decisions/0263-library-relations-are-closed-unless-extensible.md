# 0263. A library's relations are closed, unless declared `extensible`

Status: built (audited 2026-11-23). Decided (2026-11-15). Builds on 0089 (a library is held once; a program that adds
to a library relation asks of it) and 0246 (a pack is linked from module images).

## Intro

A library declares relations and the rules that compute them. Until now any file that included a library could add
rules or facts to any of its relations. A rule pack could write `unparen(a, b) :- …`, and from then on every rule in
that run that reads `unparen` saw the change, including rules that had never heard of the pack. Two problems follow:

- **Behaviour.** What a library relation holds depended on which other files happened to be loaded with it. A rule
  tested alone could answer differently inside a pack.
- **Cost.** A library is lowered, linked and prepared once for every program that includes it (0089, 0246). A program
  that adds to a library relation breaks that: the library rules reading the relation, and everything downstream of
  them, have to be computed again for that program (its *request cone*).

So a library now says which of its relations are open to the files that include it:

```
// python/compare.rr: a request relation, written by the programs that ask for comparisons
.decl cmp_same_ask(a: node, b: node) extensible
.decl cmp_same(a: node, b: node)
cmp_same(a, b) :- cmp_same_ask(a, b), unparen(a, x), unparen(b, y), same_node(x, y).
```

```
// a pack: allowed, `cmp_same_ask` is extensible
cmp_same_ask(x, y) :- pair(_, x, y).
// refused: `unparen` is declared in python/ast.rr, and not `extensible`
unparen(a, a) :- kind(a, "call").
```

## The rule

1. A file (a library file or a program) adds no rule or fact to a relation that another file declares, unless that
   declaration carries the word `extensible` after its columns.
2. Files that include each other are one module (0246 §1), and may write each other's relations.
3. The check runs when the text is read: once per library file when the library is split into modules, and once per
   program on its own statements. A refusal names the file, the line, the relation and the file declaring it.
4. `extensible` changes nothing else: lowering accepts the word and ignores it.
5. Files with no `#include` are read as a whole program and are not checked: they declare everything they use.

## What was marked

A survey of every `.rr` file in the repository that includes the library found 56 relations written from outside
the file declaring them; with the three the runner gives (`env`, `option`, `option_item`), 59 are marked `extensible`, so behaviour is unchanged. They fall into kinds:

- **Requests** (0089 §3): `cmp_same_ask`, `c4_builtin_ask`, `seen_ask`, `sort_ask`, `noqa_ask`, `vmut_ask`,
  `token_text_ask`, `upper_first_ask`, `ref_same_ask`, `global_at_ask`, `nat_ask`, `prec_ask`, `top_assign_ask`,
  `air_kind_ask`, `la_ask_*`, `repr_want`.
- **Hooks a language fills in**: `cc_boundary`, `cc_decision`, `cc_name` (complexity), `embed`, `embed_value`, `load`
  (core), `layout_at`, `py_own` (phase 1's sources).
- **Configuration and tables a pack gives**: `limit_rule`, `lit_repr_threshold`, `allow_entry`, `allow_sig`,
  `ratchet_count`, `ratchet_sig`, `la_set`, `confusable_text`, `async_api_target`, `air_needs_guard`, `env`, `option`,
  `option_item` (given by the runner).
- **A library adding to another library's relation**: `rotgt`, `star` (python/imports.rr and names.rr), `len_of`,
  `len_val`, `len_want` (pytest-collect.rr into lengths.rr), `php_dynamic` (blade into php), `param_slot`,
  `api_known`, `node_global`, `node_module`, `module_global`, `trace_member`, `trace_module`, `touch_pair`,
  `traced_global`, `node_estree_kind`.

The last group is where the hazard this decision is about lives: a relation's contents depend on whether a second
library was included. They are marked so that nothing changes today. Each should be reviewed: either the rule moves
into the declaring library, or the relation is split into a closed part and an open request.

## Consequences

- A library's closed relations are the same in every program and every pack. Their lowering, hiding and
  preparation can be done once and kept with the library's image (0246 stages 4 and 5). Only the cones of
  `extensible` relations need per-program work.
- A new write into a library relation is a reading error until the library's author opens the relation. Opening one
  is a visible change, in the library, in review.
