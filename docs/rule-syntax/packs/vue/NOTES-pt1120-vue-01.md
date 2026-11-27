# pt1120-vue-01: four eslint-plugin-vue rules over the component's view

Rules: no-watch-after-await, require-valid-default-prop, return-in-computed-property, valid-v-for. All four are
`same` as their JSON originals on both corpora (fixtures, 773 files; samples-vue, 141), with
`ptcheck --ref tbelanger/pack-translations-1120 … --pack vue --rules <the four>`.

Ground truth: ESLint 9.39.5, eslint-plugin-vue 10.11.1, vue-eslint-parser 10.4.1 (`@typescript-eslint/parser` for
`lang="ts"`, the template read by the script's parser), each rule alone, on a fleet box. Over the whole fixtures
corpus ESLint and each translation report the same (path, line) keys: 5, 70, 56 and 46. Each rule's new
`pt1120_*` test holds the counterexamples below, with ESLint's lines as its `exactly` expectations; all pass under
`rerail-engine test-rr` (the line's engine, `rerail-engine-1120`), as do no-async-in-computed-properties' tests. The
four also run with `RERAIL_STRICT_CROSSINGS`, `RERAIL_STRICT_NODES` and `RERAIL_STRICT_POSITIONS` set, with the same
findings: nothing crosses to the cross-file part.

## Why they differed

| Rule | Before (fixtures; samples-vue) | Cause |
|---|---|---|
| no-watch-after-await | 5 original-only | read the scripts through `parsed` guests (vue/component), which the engine never makes |
| require-valid-default-prop | 70 original-only | the same |
| return-in-computed-property | 56 original-only | the same, and vue/flow's `vlast_return` orders by `visit_index` |
| valid-v-for | 5 original-only, 48 translation-only; 17 translation-only | the alias and `:key` checks read vue/code's `vue-for-alias` and `vue-expression` guests: no alias was ever checked, and every `:key` was "not using the v-for's variables" |

The three script rules now include vue/vue-01 and read the view (vue/view, decision 0222) as vue-01's rules do.
valid-v-for reads the view's v-for arrows, `(SRC), (ALIAS) => { E };`, naming `guest` in its clauses as vue-02's
rules do; its directive checks (argument, modifiers, empty value, empty aliases, custom elements) still read the
`.vue` tree.

## Per rule

No rule follows the JSON against ESLint: on the corpora they agree, and on every counterexample below ESLint and the
translation agree. The comparison with the JSON on the counterexamples was not run (the suite compares on the
corpora only).

- **no-watch-after-await.** isMaybeUsedStopHandle's parents: a declarator, an assignment (`||=` too), a call's
  argument or callee (`watch(a, f)()`), a property or an array element keep the handle; `new Use(watch(a, f))`, a
  parenthesised statement and `await watch(a, f)` do not, as in ESLint. The visit order is the start's line and
  column (`earlier` has no row over the view): an await in the call's own arguments, `w(await x, f)`, comes after the
  call. `import * as V from 'vue'; V.watch()` and `watch as w` count; `<script setup>`'s top-level await is not
  checked (onSetupFunctionEnter is the setup option only).
- **return-in-computed-property.** The last return is the last by line and column. Agreed with ESLint on a switch
  with a default, try/finally, `while (true)`, do-while, `for (;;)` with a break, a labelled break, `return;` last,
  a throw only, a nested arrow's return, an accessor's setter, `computed({ get: () => {} })`, `computed(function () {
  return })` and `import { computed as c2 }`.
- **require-valid-default-prop.** Unchanged but for the view: `bad` is the old rule, and `invalid` seeds it once per
  grammar. Agreed with ESLint on every source of a default (`default`, `default() {}`, withDefaults, destructuring,
  defineModel with and without a name), `as PropType<X>`, `[String, Number]`, BigInt, `(1)`, regex, template string.
- **valid-v-for.** The v-for's variables are its view arrow's parameter names, destructured ones included; a `:key`
  reads the identifiers of its own view code, less the parameters of its own functions (`:key="ids.find(item =>
  item)"` under `v-for="item in xs"` is reported, as ESLint does), and the same-name `:key` reads its name. An alias
  in slot 1 or 2 is the view parameter whose text is that slot's item (holes counted on the alias text), so a
  multi-line alias is reported on its own line (`(a,⏎ [c])`). A keyless `<template>` hands its check to its child
  elements unless their own v-for's list reads its variables. The messages for empty aliases stay the old ones.

## Found on the way (engine and library)

- **A view node is read in the view's grammar only when its own column is seeded.** A relation read with a node
  column that `vjs`/`vts` (or a `guest` atom naming the grammar) ties to the view is copied for that grammar with
  every relation it reads; another node variable of the seeding clause keeps Vue's kinds. Counterexample, over
  `async setup() { await load(); watch(foo, f) }`:
  `d(w) :- watcher(c, f), vjs(c), nearest(w, f, vfn_kind), kind(w, "await_expression").` gives nothing;
  with `vjs(w)` it gives the await. So each rule seeds one node per clause and moves the rest into relations it
  reads with that node (`invalid` → `bad`, `no_return` → `lacks_return`, `awaits`).
- **host_node is the innermost node:** a self-closing element's v-for arrow is placed at its `self_closing_tag`,
  which spans the element's bytes. `<Iteration v-for="i in items" :key="i" />` was "not using the v-for's
  variables" until valid-v-for read `host_node(arr, h), (h = f ; vel_tag(f, h))`.
- **A relation declared with no clause is read as empty without a word** from run-policies (an editing slip here
  dropped `watcher`'s clauses; the rule then reported nothing, exit 0). A warning for a declared, never-defined
  relation that is no input would have caught it.

## Library changes wanted (existing files, not edited here)

- **vue/flow.rerail: `vlast_return` by line and column**, not `visit_index` (no row over the view);
  return-in-computed-property keeps its own `later_return` until then.
- **vue/view.rerail: an element's host node** (`vel_host(e, h)`: the element or its self-closing tag), for every
  rule that joins a view arrow to its element.
- **vue/code.rerail's template relations over the view** (`vfor`, `vfor_alias`, `tvar`, `tref`, `tbind`): they read
  rrlab's `parsed` roles and are empty under the engine, as NOTES-vue-02 says. valid-v-for's `for_var`, `reads` and
  `local` are the view's versions of `tvar` and `tref` for one directive.
- No `.decl` is needed: nothing here asks for a new primitive.
