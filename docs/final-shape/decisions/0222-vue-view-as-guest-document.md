# 0222: A Vue component's view is a guest document

(Numbered 0202 when drafted.)

Status: decided (2026-11-14, in the work to run main's Vue rules directly: "the Vue view must be a guest
document", "reuse main's view builder").

## Intro

Main checks a `.vue` file's script and template as one JavaScript program. Its view builder (`vue::view`) writes
that program, the view, out of the component:

```
<template><div v-for="x in xs">{{ x }}</div></template>      // the .vue file
<script>export default { data() { return { xs: [] } } }</script>

// the view main's builder writes (shortened): script first, then the template as code
export default { data() { return { xs: [] } } }
for (const x of xs) { x }                                     // each token placed at its .vue token
```

Main's four Vue rules (`no-dupe-keys`, `no-template-shadow`, `no-unused-refs`, `no-unused-vars`) read that
program through `embedded: script-block` and map a view node back to the `.vue` node it stands for with
`host`. The logic engine had no view. This decision gives it one, through the guest mechanism every other
embedded language already uses (decisions 0055, 0058), not through an evaluator of its own.

## Decision

1. **The view is a guest, asked for by a library rule.**
   - `vue/view.rr` holds one rule. It asks every primary `.vue` document for its `script-block` program:

     ```
     // embed(document, at, grammar, role): the guest request of decision 0055
     embed(d, r, "script-block", "program") :- doc_role(d, "primary"), doc_root(d, r).
     ```

   - A grammar name may be an embedding's registry id (`Via::id()`: `script-block`, `compiled-view`) as well
     as a dialect label (`language::is_grammar`).
2. **Main's builder writes the text.** `Guests::program` re-parses the host, calls the embedder main
   registered for the id (`embed::embedder(via).build`), and parses the text in the dialect the builder chose:
   `javascript`, or `typescript` for `<script lang="ts">`. The guest's grammar is that label, so rules find it
   with `guest(g, _, _, "javascript")`. A view is hidden from rules that do not ask for it (decision 0058 §1).
3. **Its nodes are placed through the builder's segments.**
   - The view's placement is `Placement::Segments`, the builder's `SegmentMap`.
   - A finding's position is the segment map's primary file range. `invalid-000.vue` reports at line 1,
     column 43, as main does.
   - `host_node(n, h)` uses the hull of all the file ranges (`SegmentMap::bounds`). A view node spread over
     several template tokens maps to the `.vue` node that covers them all.
4. **The view keeps its tape**, so W's values (`value`, a key's text) read over it as over a file. Only a
   program guest keeps one; string and annotation guests do not.
5. **The translator writes the view terms with guest atoms.** Two terms change, and both are in
   `json_rules/embedded.rs`:
   - A clause with `embedded: script-block` starts with `docof(v, g), guest(g, _, _, <dialect>)`. There is one
     alternative per dialect the embedding may produce, as main's `EmbeddedTest` compiles once per dialect.
   - `host` becomes `host_node(v, h)`.
   - `embedded` (a view node standing at a `.vue` node) becomes a helper rule. It has one body per dialect.
6. **`compare` reads a key's name and an unquoted text as main does.** The translator wrote every text mode
   but `node` as `value`, and a method's key or an identifier has no value. For the ECMAScript languages and
   Vue, three relations in `tsx/compare.rr` now stand in:

   ```
   cmp_key(n, x)        // text: "key"      ESLint's getStaticPropertyName:  { foo() {} }  "foo"
   cmp_unquoted(n, x)   // text: "unquoted" the text, outer quotes off:       'foo'  foo   both "foo"
   cmp_value(n, x)      // transform value: true   the value, else the name:  foo   "foo"
   ```

   In a clause over an embedded program, a transform's source bound by a util the rule calls, such as
   `matches: item-P` binding K, now counts as bound. Before, `unset(KT)` was written beside `KT`'s
   definition, and the rule could never hold.
7. **Hiding specialises library relations for the view's dialect.**
   - Kind constants resolve in the run's dialect, which is `vue`.
   - A rule over view nodes therefore qualifies its kind and field constants (`javascript:arrow_function`). This
     covers negated ones too.
   - A kind-set fact (`fn_scope(k)`) gets a qualified copy, `rr_in_<g>_rows_<rel>`.
   - A library relation reached through `host_node` is specialised per grammar, as relations reached through
     `guest` already were.

## Results

Main's four rules, translated and run together over the 156 eslint-plugin-vue fixture files of their
four directories. Main conforms to ESLint on these rules (`packs/vue/conformance.json`, no divergences), so
ESLint's findings are the expected ones. Every rule runs over every file. A finding is keyed by
(rule, path, line).

| rule | expected | got | match | missing | extra |
|---|---|---|---|---|---|
| no-dupe-keys | 35 | 50 | 30 | 5 | 20 |
| no-template-shadow | 19 | 21 | 19 | 0 | 2 |
| no-unused-refs | 7 | 12 | 7 | 0 | 5 |
| no-unused-vars | 81 | 81 | 74 | 7 | 7 |

The run takes 4.3 s CPU and 325 MB at peak. The misses and extras are open below. None found so far is a view
placement: each one reproduces over a plain `.js` file, or is a rule term.

## Open

- no-dupe-keys, 5 missing: `data: () => ({ foo: null })`. The translated `inside` with `field: body` asks
  for `field(_, "body", n)` of the object itself, and tree-sitter puts a `parenthesized_expression` between them.
- no-dupe-keys, about 15 extra: a getter and setter pair (`get foo() {}`, `set foo(v) {}`) is one key to ESLint.
  The rule tells them apart by a `get`/`set` child, and an anonymous token is not a `child` row. A `.js` file
  shows the same.
- no-unused-refs, 5 extra: `for (const k in this.$refs)` and `defineComponent` forms use every ref.
- no-unused-vars, 7 missing and 7 extra; no-template-shadow, 2 extra on one `<script setup>` file. These are
  not traced yet.
- `cmp_key` reads an identifier key's text as written. Escapes in an identifier are not decoded.
- A util-bound transform source counts only over an embedded program. Counted everywhere, it changes nine
  Python rules from `unset(T)` to T's definition: air001, air301, fast001, icn001, pt019, ruf007, ruf022,
  ruf023 and up035. Those need the Ruff comparison first.
- `cmp_key` and `cmp_unquoted` are used for the ECMAScript languages and Vue only. Python and the other
  languages still read `key` and `unquoted` as `value`.

- A `Segments` placement composes with a string placement only coarsely (`Placement::then`), so a guest inside
  the view is placed at its whole host range.
- Placeholders the builder omits from the view are still nodes of the `.vue` tree. A rule that walks the host
  sees them.
- `blade/view.rr` asks for Blade's `compiled-view` in the same way. No fixture run covers it yet.
- A view node's extent is its token hull. Main reports some nodes at their first token only.
