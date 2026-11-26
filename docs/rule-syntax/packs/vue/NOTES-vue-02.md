# vue-02: five rules over the component's view

Rules: no-setup-props-reactivity-loss, no-side-effects-in-computed-properties, no-template-shadow, no-unused-refs,
no-unused-vars. All five are `same` as their JSON originals on both corpora (fixtures, 773 files; samples-vue, 141).
The check was `ptcheck … --pack vue --rules <the five>`, and a full key diff of both sides on the same corpora.

## Why they differed

The earlier versions read a Vue file's code through rrlab's guests: `parsed(v, "vue-expression", g)` and the
other roles that `lib/vue/code.rerail` and `lib/vue/component.rerail` document. The engine makes no such guests.
It makes one, the view of decision 0222: the scripts and the template's expressions as one ECMAScript program.
With nothing to read, four rules reported nothing (original-only). no-unused-refs reported every ref that a script
read (translation-only). The five rules now read the view, as the JSON originals do (`embedded: script-block`):
`docof(n, g), (guest(g, _, _, "javascript") ; guest(g, _, _, "typescript") ; guest(g, _, _, "tsx"))`, and then
host_node back to the `.vue` tree. Their opening comments still describe ESLint's rule.

`translate-json` gives a mechanical version of each original. Its output differs too: reactivity-loss 31
original-only, no-unused-vars 11 original-only, no-unused-refs 6 translation-only, and side-effects is refused as
ungrounded. Each cause is listed under "Engine and translator gaps".

## Per rule

Each rule follows the JSON. Nothing differs, so the JSON is never set aside. ESLint 9.39.5 with eslint-plugin-vue
10.11.1 (npx on a box, typescript-eslint's parser for `<script lang="ts">`) agrees with both sides on the cases below,
line and column.

- **no-unused-vars.** A destructured alias is checked even when a later one is used:
  `<div v-for="({a, b}, [c, d], e, f) in foo">{{f}}</div>` reports a, b, c and d, and e passes. tsx/names binds
  none of them over the view (see below). A TypeScript view's `(item, i) =>` binds nothing there either. So
  `vv_param` and `vv_site` read the binding positions themselves.
- **no-template-shadow.** `const { j, x: [k] } = useThing()` in `<script setup>`: `v-for="j in 5"` and
  `v-for="k in 5"` are reported. These are top-level names a pattern binds (`vv_module_name`).
  `defineComponent({ props: ['item'] })` imported from 'vue' is a component definition. Its import source is read
  with `str`, because `value` has no rows over the view.
- **no-unused-refs.** `useTemplateRef('x')` reads x: a first argument is `(` before it, read through
  `structural_before` (`prev_token` has no rows over the view). `type T = number` in `<script setup>` reads
  `ref="T"`, as main's `binds: {scope: module}` does.
- **no-side-effects-in-computed-properties.** `const { s } = useStore(); computed(() => { s.count++ })` is reported.
  s is declared by a pattern, so ref_site never reaches it; `vv_site` does. In a computed() getter a mutator call's
  object counts as it comes (`computed(() => arr.reverse())`). In an options getter it must be a member access
  (`this.arr.reverse()`), as in the JSON's `callee-mut-comp` and `callee-mut-opt`.
- **no-setup-props-reactivity-loss.** `setup(props) { const a = props.a }` and `setup(props: P) { … }`: the props
  parameter is found through its occurrence, the declared name included, as main's `binds.occurrence` reads it.
  `const { c, d: dd = 2, ...rest } = defineProps(…)`, then `c`, `foo(dd)`, `rest.q` at the top level: all reported.

## Library: lib/vue/vue-02.rerail (new)

This file holds what two or more of the rules share: the template's arrows and parameters (`vv_arrow`, `vv_param`,
`vv_destructured`, `vv_read`), name resolution over the view (`vv_site`, `vv_site_only`, `vv_pattern_name`), the
scripts' top-level names (`vv_module_name`), and `<script setup>`'s statements (`vv_setup_stmt`, `vv_marker`). Each
rule keeps its own version of "the component's definition object": the three JSON originals differ in it (which
argument, which definers, an `as`/`satisfies`, `// @vue/component`).

## Library changes wanted (existing files, not edited here)

- **tsx/names.rerail.** Some atoms test a kind through a variable: `bound_name` (`kind(p, pk), (pk = "…" ; …)`),
  the type and interface `decl_site`, `param_name`'s required/optional parameter, and the function-expression name.
  Write them with literal kinds, `(kind(p, "a") ; kind(p, "b"))`. The view's dialect specialises a kind written in an
  atom, not one compared with `=`, so over the view these declare nothing today. `{ a }` in a pattern is then a
  reference, and a TypeScript parameter declares nothing. Once that is fixed, `vv_site`, `vv_pattern_scope` and
  `vv_module_name`'s pattern and type branches can go.
- **lib/vue/code, component, mutation and template's code relations** read rrlab's `parsed` roles, which the engine
  never makes, so they are dead in engine runs. The other vue rules built on them (no-dupe-keys, no-mutating-props,
  no-ref-as-operand, require-valid-default-prop, return-in-computed-property, no-async-in-computed-properties,
  no-export-in-script-setup, no-watch-after-await, valid-v-for) need the same move to the view. vue-02's
  resolution relations could serve them.

## Engine and translator gaps

- The view's kinds are specialised only in a rule body that names `guest` itself, and in what that body reaches. If
  a library relation does the test (`vv_doc(g) :- guest(g, _, _, "javascript") ; …`) and the rule joins it through
  `docof(n, g), vv_doc(g)`, the rule finds nothing: no-unused-vars went from 138 findings to 0.
- Over the view, `value(n, v)` has no rows for strings (`str` has them), and neither do `prev_token` and `token`. `op`,
  `structural_before`, `structural_after`, comments and `end_line` work. The translator writes `value` for the
  `vue|@vue/composition-api|…` source tests, so its outputs take a Vue import for a foreign one.
- The translator's side-effects output is refused as ungrounded (its line 539: the `@vue/component` alternative,
  `not: {not: …}`, nests a count over an outer variable). It also applies the `constraints` on T after `unset(T)`,
  which would drop every mutation but Object.assign's.
- A `.decl` is not needed: nothing here asks for a new primitive.
