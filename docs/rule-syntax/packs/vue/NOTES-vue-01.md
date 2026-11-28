# vue-01: five eslint-plugin-vue rules against their originals

Rules: `no-async-in-computed-properties`, `no-dupe-keys`, `no-export-in-script-setup`, `no-mutating-props`,
`no-ref-as-operand`.

Ground truth: ESLint 9.39.5 with eslint-plugin-vue 10.11.1 and vue-eslint-parser 10.4.1 (`@typescript-eslint/parser`
for `lang="ts"`), each rule alone at "error", on a fleet box.

## Results

| Rule | Before | Now | fixtures (both) | samples-vue (both) |
|---|---|---|---|---|
| no-async-in-computed-properties | 92 original-only | same | 92 | 0 |
| no-dupe-keys | 69 + 1 original-only | same | 69 | 1 |
| no-export-in-script-setup | 15 original-only | same | 15 | 0 |
| no-mutating-props | 64 original-only | same | 64 | 0 |
| no-ref-as-operand | 43 original-only | same | 43 | 0 |

Each rule's own fixture directory was also run through ESLint: every (path, line) key matches (async 92, dupe-keys 35,
export 15, mutating-props 64, ref-as-operand 43).
Counterexamples outside the fixtures, ESLint and the translation agreeing on each:

    <script setup> const n = ref(0) </script> <template>{{ n + 1 }}</template>      ref-as-operand: the script's n + 1
                                                                                     only; a template unwraps refs
    <script> const foo = 1 </script> <script setup> defineProps(['foo']) </script>   dupe-keys: line 2 (one program)
    v-model="title", v-model="props.count", v-for="item in items" @click="item = 1"  mutating-props: the first two
    <script setup lang="ts"> export type T …; export interface I …; export const x   export: the const only
    lang="ts": computed { a(): Promise<number> { return f().then(…) } }, computed(async () => 1)   async: both

No rule follows the JSON against the tool: where both were checked they agree.

## Why the translations reported nothing

vue/component.rerail reads each `<script>` through `parsed(raw, g, p)`, a guest per block, as rrlab gave them. On
main's engine that gives nothing:

- A guest is made only where a rule names its grammar as a literal (`embed`, or `parsed` with a constant grammar);
  vscript's grammar comes from a disjunction, so no block was ever parsed.
- Asked for with `embed`, a block's guest keeps no tape (`guests.rs`: only a program guest does), so `value` has no
  row there.

So the batch's rules read the component's view instead (vue/view.rerail, decision 0222): the program the engine
builds for a `.vue` file, scripts in source order and then the template's code, in the scripts' `lang`, placed at the
file's bytes. vue-eslint-parser also reads the scripts as one program. The template half of no-mutating-props
(free names, `this`, v-model and `.sync`) is read from the view's template statements, as vue/code's guests are no
longer made.

## Library: lib/vue/vue-01.rerail (new)

Shared by all five rules:

- Cost: each clause that starts a search is written once per grammar, so no-mutating-props is 180 lines (141
  before); the `vts` twins go once engine gap 2 is fixed.
- `vjs(n)`, `vts(n)`: a node of a JavaScript or TypeScript view. A rule seeds the relation its search starts from with
  one of them, one clause per grammar (engine gap 2).
- `vblock(n, se)`, `vtop(e, setup)`, `vview_root(p)`: a view node's script block, a block's top-level statements, the
  module scope.
- Clauses added to vue/component's script relations over the view: `vin_script`, `vin_setup`, `vsetup_doc`, vcomp's
  `export default`, `vmacro_site`.
- `vvalued`: reads `value` of no particular node, which keeps `value` an input made for every node (engine gap 3).
- Restated library clauses the engine cannot specialise: `vlater_setter` by line and column (gap 6), tsx/names'
  `pattern_name`, `bound_name` and the type alias / interface `decl_site` with kind atoms (gap 5).

Proposed changes to existing libraries, so that vue-01 can shrink to the seeds:

- vue/component.rerail: read the scripts from the view (vblock, vtop) rather than per-block `parsed`; `vlater_setter`
  without `earlier`. (`vopt_fn`'s pair clause now ties h's value to v in component.rerail itself, gap 4.)
- tsx/names.rerail: `kind(x, k), (k = "a" ; k = "b")` as `(kind(x, "a") ; kind(x, "b"))` (pattern_name, bound_name,
  import_spec, the named function expression's and the type alias / interface decl_site, write_target, plain_param,
  escope_node, erased).

## Engine gaps (main, ~/pt/bin/rerail-engine)

1. **Guests only by literal grammar; block guests have no tape.** As above. A `.decl` is not the fix: vue/component
   should read the view.
2. **A guest node's kinds are named in its grammar only when the rule reached the node from a guest relation naming
   it.** A disjunction `(vjs(n) ; vts(n))` names neither, and a derived column filled from both grammars (getter's f in
   async-in-computed) names neither; each clause has to be written once per grammar. A `lang="tsx"` view is not
   seeded at all (no corpus has one).
3. **`value` on request is never asked for a guest node, and a copied relation that reads it loses every row.**
   Minimal case, over the view: `k(n, x) :- kind(n, "property_identifier"), text(n, x). k(n, x) :- kind(n,
   "string"), value(n, x).` read as `vjs(n), k(n, _)` gives nothing, from either rule. key_static (through
   static_string) and everything above it was empty; under `RERAIL_NODE_SOURCES=eager`, key_static, vopt and `value`
   over the view all have rows. Likely cause (not traced): the
   request rules (`rr::lower::sources_on_request`) are built from the readers' literals before guest hiding copies them.
4. **A head variable that no base atom ties to the seeded column keeps the run language's kinds.**
   `vopt_fn(h, v, v) :- kind(h, "pair"), function_node(v)` copied for a seeded v reads `kind(h, "pair")` as Vue's:
   `data: () => ({ foo })` and `setup: () => ({ … })` declared nothing.
5. **`kind(x, k), (k = "a" ; k = "b")` is not named in the guest's grammar** (a plain `k = "a"` is): destructured
   names (`const { a } = toRefs(p)`, `let [m] = defineModel()`, `setup(_, { emit })`) declared nothing.
6. **`earlier` / `visit_index` have no row over the view** (react-04 found the same for TypeScript files).
7. Minor: `cat("c", c)` with c from `col(n, c)` printed unrelated interned strings in a probe; the rules don't do it.

## Review fixes (vue-01.review.md)

All 21 defects fixed; none disputed. Each counterexample is a `.test` of its rule (22 tests, all passing under
`rerail-engine test-rr`), with ESLint's (eslint-plugin-vue 10.11.1) lines as the expected findings. Over the
reviewer's 45 files and 4 more of mine (an accessor's setter, `:[a].sync`, `{ "emit": e2, emit = … }`, the
ignoredObjectNames wrappers), ESLint and the translations report the same 84 (file, rule, line) keys.

Library, vue/component.rerail (shared with no-side-effects-in-computed-properties, return-in-computed-property,
no-template-shadow and no-watch-after-await, whose originals use the same utils):

- `vcomputed` follows utils.getComputedProperties: a method or accessor (`get x()`, and a setter's body too, as
  ESLint reports it), or a FunctionExpression value without `as`, or the `get` FunctionExpression of an object; an
  arrow is no getter. `vgetter` stays computed()'s (getGetterBodyFromComputedFunction takes arrows).
- `vvue_import_call` reads the imported name (`import { computed as c }` calls computed), as ReferenceTracker does;
  a default or namespace binding called directly is none.
- `vname` (iteratePropertyValue) and `vopt_fn` read an option's value through parentheses only: `props: ['a'] as any`
  declares nothing.
- `vprop_name` flattens defineProps<T>'s type as flattenTypeNodes: unions, intersections, parentheses, and a type
  name resolved (ref_site) to its interface's body or its alias's type.

Rules:

- no-async-in-computed-properties: an async getter is reported only when the reported function starts on its body's
  line (verify's line test); `$nextTick` on a `const vm = this`; the ignoredObjectNames root skips `!`, `as`,
  `satisfies`, `<T>x` and `x<T>` (skipWrapper). The option is read through `ignored_name`, which a test can give.
- no-dupe-keys: type aliases and interfaces are module-scope variables (decl_site's type clause restated with kind
  atoms in vue-01, gap 5); one report per name, at its first declaration (defs[0]); an initialiser that reads the
  props through `{ props }` (a shorthand property) skips the name.
- no-export-in-script-setup: any `export declare …` is a type export.
- no-mutating-props: v-model's and `.sync`'s dynamic argument is checked as their value is (the view writes it as
  its own statement at the `[…]`); type aliases leave the template's props through the restated decl_site.
- no-ref-as-operand: makers by imported name; a maker under `as` makes no ref variable (`deparen`, not `vunwrap`);
  toRefs binds only the pattern's top-level properties; an event name may be a template without `${}`;
  `{ emit: e }` and a defaulted context (`ctx = {}`) are setup's; a variable declared with defineModel() is remade
  only by another defineModel(), as the plugin processes defineModel() calls after every ref() call.

Option tests: run-policies takes no options, so ignoredObjectNames was checked with `run-rr … options.json`
(`{"ignoredObjectNames": ["z"]}`) against ESLint with the same option: both report x4-ign.vue:5 only.
