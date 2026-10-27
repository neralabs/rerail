# Rerail rule pack: Vue, candidates

The ports in `rules/` here are still being evaluated: scored against the tools' expected findings and reported, never
gated and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

3 differ from upstream on cases no single-file, type-free read can answer as upstream does (a prop type imported from
another file, a switch the TypeScript checker finds exhaustive, a type the checker infers, a TypeScript
`<script setup>` without `lang`), 1 fails `compact` (`no-dupe-v-else-if`) and 1 names words found in upstream's tests
but not its code (`no-unused-refs`).

Over all 24 ports, shipped and candidate: every one agrees with eslint-plugin-vue on every fixture case, 1,106 findings
on the 773 fixture files, with no false positive or false negative and no declared divergence.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `eslint-plugin-vue-no-dupe-keys`

Ports eslint-plugin-vue 10.11.1 `vue/no-dupe-keys` with default options (groups props, computed, data, methods, setup). Two clauses over the script-block program (`ast.embedded`; a script's marker, the `empty_statement` whose host is the block's start tag, tells `<script setup>` statements apart). `options-api`: in a component object (`export default {...}`, the last object argument of `Vue.component`/`mixin`/`extend`, `X.component`/`X.mixin`, `defineNuxtComponent`, and `component`/`createApp`/`defineComponent` when global or imported from vue, `new Vue({...})`, or an object on the line after a `@vue/component` comment), each name of the `props` array/object, `computed`, `data`, `methods` and `setup` groups (object keys of the group value, of the object a group function returns at its top level, or of an arrow function's expression body; spreads and non-static keys ignored) that an earlier name of the same component already holds is reported at its key, with upstream's getter/setter pairing (a setter paired with a getter of the same name is not a name). `script-setup-props`: a top-level `<script setup>` `defineProps` (alone or in `withDefaults`, as a statement or a declarator initializer) prop name (array element, object key or inline type-literal member, not renamed in a destructuring pattern) whose module-scope binding (variable, function, class, import) is not initialised from the props (a declarator whose initializer holds the `defineProps` call or reads a variable the same declarator binds) is reported at the first binding's declaration node (declarator, function/class declaration, import specifier or clause). Not read: props declared by a referenced interface or type alias, props named by a const identifier, computed group keys, `require('vue')` helpers, `export default` inside `<script setup>` (upstream skips it).

### `eslint-plugin-vue-no-dupe-v-else-if`

Ports eslint-plugin-vue 10.11.1 `vue/no-dupe-v-else-if` with default options. Reads the script-block program's (`ast.embedded`) `if (...) { } else if (...) { }` chain (v-if / v-else-if elements, nested chains stay separate; the `true` written for a condition-less directive, whose host is its directive name, is no condition): for each else-if test it applies eslint-plugin-vue's or/and set-cover (the test and, when it is a `&&`, each flattened `&&` operand are the conditions; a condition is dead when every `||` operand of it is a superset, as a set of `&&` operands compared by tokens with `||`/`&&` operands in either order, of an `||` operand of some earlier test in the chain) and reports the first condition to become dead walking the chain from the nearest earlier branch, operands in source order and the whole test last, at that condition (parentheses excluded, as ESTree).

### `eslint-plugin-vue-no-unused-refs`

Ports eslint-plugin-vue 10.11.1 vue/no-unused-refs (no options): a static template `ref="name"` attribute is reported at its value literal (quotes included) unless the file uses the name, or has an unknown use of $refs. Used: `$refs.name`/`$refs['name']`/object-pattern destructuring of `$refs` (script `this.$refs` and bare `$refs`, template bare `$refs`), `useTemplateRef('name')`, a name `setup()` of a Vue object (export default, defineComponent/Vue.component/..., new Vue) returns, and, when the file has a <script setup>, any module/global binding of the program. Unknown use (`const a = $refs`, `a = $refs`, `f($refs)`, `return $refs`, `for (k in $refs)`, dynamic key or rest/computed destructuring) suppresses every report in the file. A clause over the file's own tree: the uses are read through `embedded` from the document in the script-block program (syntax, scope binds), each name compared with the attribute value through `host: { compare }`.

### `eslint-plugin-vue-require-valid-default-prop`

Ports eslint-plugin-vue 10.11.1 `vue/require-valid-default-prop` with default options: a prop's default must match the native constructors (String, Number, Boolean, Function, Object, Array, Symbol, BigInt) its `type` names; Object, Array and Function values must come from a factory, whose returns are checked too. Reads Options-API component objects (export default outside script setup, Vue.component/mixin/extend, component/createApp/defineComponent/defineNuxtComponent calls, new Vue, `// @vue/component` marks), top-level script-setup defineProps({...}) objects with withDefaults defaults and destructured defaults, and defineModel options. Props typed only by a TypeScript type argument (defineProps<T>(), defineModel<T>()) and a defineProps argument held in a const are not read.

### `eslint-plugin-vue-return-in-computed-property`

Ports eslint-plugin-vue 10.11.1 `vue/return-in-computed-property` (default options, treatUndefinedAsUnspecified: true). Reports a computed getter that can fall off its end or whose last `return` has no value: an Options API getter (`foo() {}`, `foo: function () {}`, `foo: { get() {} }`, `foo: { get: function () {} }`) of the `computed` object of a Vue object (`export default` outside `<script setup>`, `Vue.component`/`mixin`/`extend`, `app.component`/`mixin`, `defineComponent`, `createApp`, `defineNuxtComponent`, `new Vue`, or an object on the line after a `@vue/component` comment), and a composition getter given to `computed(...)` imported from `vue`, `@vue/composition-api`, `@nuxtjs/composition-api` or `#imports` (a function, or `{ get }`). Reads ESLint's code-path end reachability as the recursive `nc` utility (return, throw, break, continue, blocks, if/else, switch with a default, try/catch/finally, labelled statements, loops with a constant truthy test) and the last `return` statement in source order. Reported at the function (a method at its parameters).
