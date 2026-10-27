# Rerail rule pack: Vue

Ports of 24 eslint-plugin-vue 10.11.1 rules for Vue single-file components, with their default options. 19 ship, each
passing the conformance check ([conformance/README.md](../../conformance/README.md#the-gate)), judged against
eslint-plugin-vue's own findings on its RuleTester cases (`upstream/eslint-plugin-vue/fixtures.expected.json`) and
against the tool's labelled real-code samples: 18 on, as `flat/recommended` turns them on, and
`no-setup-props-reactivity-loss` off until turned on (no recommended configuration of eslint-plugin-vue turns it on).
The ports cover `.vue` files only (`rerail.in_scope`), so each upstream case it lints as a bare script is written as
the component that holds it, wrapped in `<script>` as `generate.mjs` writes its fixtures, and scored on upstream's
answer there. The other 5 are candidates under `candidates/rules/`, scored and reported but never gating a check,
until they pass: 3 differ from upstream on cases no single-file, type-free read can answer as upstream does (a prop
type imported from another file, a switch the TypeScript checker finds exhaustive, a type the checker infers, a
TypeScript `<script setup>` without `lang`), 1 fails `compact` (`no-dupe-v-else-if`) and 1 names words found in
upstream's tests but not its code (`no-unused-refs`). The Vue family has no rename variants (`holds-under-rename`
covers the ECMAScript packs), so the real-code samples are the pack's only evidence that a port is not fitted to its
tests. Each
reads the component's two documents: its tree-sitter-vue tree, and the ECMAScript program its scripts and template
expressions make (the `script-block` program a clause reads with `ast.embedded`, or reaches from a template node with
`embedded`; `host` leads back from a program node to the template node it stands for). Each port
is scored against eslint-plugin-vue's own findings on its RuleTester cases and on cases Rerail wrote for inputs a port
once misread (`fixtures/<rule>/rerail-*.vue`, listed in `upstream/eslint-plugin-vue/UPSTREAM.md`), and against the
labelled lines of real-code samples ([docs/upstream.md](../../docs/upstream.md#samples)), through the
engine's production workspace path. Every port agrees with eslint-plugin-vue on every fixture case: 1,106 findings on
the 773 fixture files, with no false positive or false negative and no declared divergence.

It is checked by:

```sh
node --import tsx conformance/languages/cli.ts --pack vue
npx vitest run test/engine/catalog/packs.test.ts -t "pack vue"
npm run test:conformance
```

([pack structure and validation contract](../../docs/pack-format.md)).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): its match definition and its 19 shipped rules, 18 on and 1 off |
| `rules/` | The 19 shipped code-rule documents (`rerail.policy/v1`) |
| `candidates/rules/` | The 5 candidate code-rule documents (`rerail.policy/v1`), scored but never shipped |
| `conformance.json` | Each port's upstream rule, how findings compare, and the capabilities it exercises; the pinned parser stack |
| `divergences.json` | Declared disagreements with eslint-plugin-vue (none) |
| `upstream/eslint-plugin-vue/` | The copied RuleTester cases (`fixtures/`: `generate.mjs`'s `<valid\|invalid>-<n>.vue`), the cases Rerail wrote (`authored.mjs`, written as `fixtures/<rule>/rerail-*.vue` and listed in `UPSTREAM.md`), each case's fate (`cases.json`), the expected findings (`fixtures.expected.json`), the lock naming its real-code samples (`samples.lock`) with their sources and licences (`SAMPLES.md`), and the upstream manifest (`gates.yaml`: its pins, approved configurations, the file types it lints and the facts of every rule the pack ports). The cases, `cases.json` and the expected findings are pack test data, not in git: one archive in S3, which `test-data.lock` names and `npm run fixtures:pull` fetches with the samples; a maintainer derives them from the pins (`npm run fixtures:derive`, `scripts/pack-data/recipes/vue.ts`) and publishes them through `upstream/eslint-plugin-vue/publish/` ([docs/upstream.md](../../docs/upstream.md#take-a-new-snapshot)). Paused: `fixtures:derive` refuses every tool-pack until the generators write per-repo provenance ([docs/upstream.md](../../docs/upstream.md#derive-and-bumps-are-paused)) |
| `licenses/eslint-plugin-vue/` | eslint-plugin-vue's licence and `UPSTREAM.md`, per upstream repository: its source, the patterns of the files taken and how (each file's provenance is `provenance.json` in the tool's test-data archive) |

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `eslint-plugin-vue-attribute-hyphenation`

Ports eslint-plugin-vue 10.11.1 `vue/attribute-hyphenation` with default options (always, ignore: [], ignoreTags: []): on a component tag or a `<slot>`, a static attribute name, or the static argument of `v-bind`/`:`/`.`/`v-model`, that contains an upper-case letter and no `data-`, `aria-`, `slot-scope` or SVG weird-case attribute name (substring test) is reported at the attribute, with its key (modifiers included) in the message. A tag is a component as eslint-plugin-vue utils.isCustomComponent decides: a static `is` attribute, `:is`/`v-bind:is`/`.is` or `v-is` (case-insensitive), or its raw tag name (case-sensitive) is not in the well-known list of its namespace (HTML, SVG under `<svg>`, MathML under `<math>`; namespace propagation follows vue-eslint-parser: foreignObject/desc/title and MathML text integration points switch children back to HTML); the element lists are the shared regexes in the `custom-tag` utility.

### `eslint-plugin-vue-no-async-in-computed-properties`

Ports eslint-plugin-vue 10.11.1 `vue/no-async-in-computed-properties` (default options, ignoredObjectNames: []). Reports, directly in the body of a computed getter (not in a nested function): an `async` getter, an `await`, `new Promise`, a promise call (`.then`/`.catch`/`.finally`, `Promise.all`/`allSettled`/`any`/`race`/`reject`/`resolve`/`try`/`withResolvers`), `this.$nextTick`/`Vue.nextTick`, and `setTimeout`/`setInterval`/`setImmediate`/`requestAnimationFrame` (bare or on `window`) with an argument. Options API getters are read in files without `<script setup>`; composition getters (`computed(...)` imported from Vue) inside a Vue object, or inside `<script setup>`.

### `eslint-plugin-vue-no-child-content`

Ports eslint-plugin-vue 10.11.1 `vue/no-child-content` with default options (directives html and text, one clause each): a paired (non self-closing) element whose start tag has `v-html`/`v-text` and whose children (between start and end tag) hold anything but whitespace-only text (text, interpolation, element, comment; the scanner keeps character references in the text, and one that decodes to a character `String.prototype.trim` strips, such as `&#32;` or `&nbsp;`, is whitespace as it is to the plugin). Reported at the child content, from the end of the start tag to the start of the end tag, as upstream reports getLocationRange of the element's children and comments (the start tag's own `>` found as HTML tokenizes it, past quoted attribute values).

### `eslint-plugin-vue-no-deprecated-v-on-native-modifier`

Ports eslint-plugin-vue 10.11.1 `vue/no-deprecated-v-on-native-modifier`: a `native` modifier token of a `v-on:`/`@` directive (a directive_modifier named native under a directive_attribute that is v-on or starts with `@`). Reported at the modifier name.

### `eslint-plugin-vue-no-dupe-keys`

Ports eslint-plugin-vue 10.11.1 `vue/no-dupe-keys` with default options (groups props, computed, data, methods, setup). Two clauses over the script-block program (`ast.embedded`; a script's marker, the `empty_statement` whose host is the block's start tag, tells `<script setup>` statements apart). `options-api`: in a component object (`export default {...}`, the last object argument of `Vue.component`/`mixin`/`extend`, `X.component`/`X.mixin`, `defineNuxtComponent`, and `component`/`createApp`/`defineComponent` when global or imported from vue, `new Vue({...})`, or an object on the line after a `@vue/component` comment), each name of the `props` array/object, `computed`, `data`, `methods` and `setup` groups (object keys of the group value, of the object a group function returns at its top level, or of an arrow function's expression body; spreads and non-static keys ignored) that an earlier name of the same component already holds is reported at its key, with upstream's getter/setter pairing (a setter paired with a getter of the same name is not a name). `script-setup-props`: a top-level `<script setup>` `defineProps` (alone or in `withDefaults`, as a statement or a declarator initializer) prop name (array element, object key or inline type-literal member, not renamed in a destructuring pattern) whose module-scope binding (variable, function, class, import) is not initialised from the props (a declarator whose initializer holds the `defineProps` call or reads a variable the same declarator binds) is reported at the first binding's declaration node (declarator, function/class declaration, import specifier or clause). Not read: props declared by a referenced interface or type alias, props named by a const identifier, computed group keys, `require('vue')` helpers, `export default` inside `<script setup>` (upstream skips it).

### `eslint-plugin-vue-no-dupe-v-else-if`

Ports eslint-plugin-vue 10.11.1 `vue/no-dupe-v-else-if` with default options. Reads the script-block program's (`ast.embedded`) `if (...) { } else if (...) { }` chain (v-if / v-else-if elements, nested chains stay separate; the `true` written for a condition-less directive, whose host is its directive name, is no condition): for each else-if test it applies eslint-plugin-vue's or/and set-cover (the test and, when it is a `&&`, each flattened `&&` operand are the conditions; a condition is dead when every `||` operand of it is a superset, as a set of `&&` operands compared by tokens with `||`/`&&` operands in either order, of an `||` operand of some earlier test in the chain) and reports the first condition to become dead walking the chain from the nearest earlier branch, operands in source order and the whole test last, at that condition (parentheses excluded, as ESTree).

### `eslint-plugin-vue-no-duplicate-attributes`

Ports eslint-plugin-vue 10.11.1 `vue/no-duplicate-attributes` with default options (allowCoexistClass and allowCoexistStyle true): in one start tag, a plain attribute or a `v-bind:NAME`/`:NAME`/`.NAME` directive with a static argument whose name an earlier attribute of the tag also has; `class` and `style` may coexist as one plain plus one bound attribute but not two of the same kind. Dynamic arguments and other directives are ignored. Reported at the later attribute.

### `eslint-plugin-vue-no-export-in-script-setup`

Ports eslint-plugin-vue 10.11.1 vue/no-export-in-script-setup (default options): every export_statement of the <script setup> block (a top-level statement of the script-block program whose nearest earlier script marker, the empty statement written before each script, has the setup block's start tag as its host), except type exports (`export type {..}`, `export type`/`interface` declarations, and `export { type A }` lists whose every specifier is type-only). A named export with a declaration (`export const a`, `export function f`) is reported at its first token, `export`, as upstream reports an ExportNamedDeclaration with a `declaration` at `getFirstToken(node).loc`; every other export (`export { a }`, `export * from`, `export default ...`) at the whole statement, as upstream reports at `node.loc`. Reads only syntax and the script markers' host start tags.

### `eslint-plugin-vue-no-mutating-props`

Ports eslint-plugin-vue 10.11.1 `vue/no-mutating-props` with default options (shallowOnly: false): a mutation of a component prop, reported at the mutating node (assignment, update, delete, call of push/pop/shift/unshift/reverse/splice/sort/copyWithin/fill, or Object.assign with the prop as first argument). Reads the script-block program (`ast.embedded`): (1) template code inside the statement block whose host is the `<template>` element: an identifier not bound by a v-for/slot arrow whose name is a prop (Options API `props` array strings and object keys of `export default {}`/`defineComponent({})`/`Vue.component/mixin/extend`/`new Vue({})`, `<script setup>` `defineProps` runtime array/object keys, `defineProps<{...}>()` type-literal and same-file interface/type-alias members that are neither a JS global from the upstream whitelist nor a binding of the script, plus the names destructured from `defineProps`) or the `defineProps` result variable, and `this.<prop>`; `v-model` and `.sync` expressions (whose parentheses' host is that directive) are reported as a whole when their member chain starts at such a name; (2) script code: `this.<prop>` (or a `const vm = this` alias) inside a Vue object that declares the prop; (3) references (outside the template) to the `setup(props)` first parameter, to the `const props = defineProps()` / `withDefaults(defineProps())` variable (a mutation needs at least one member access, as upstream), and to names destructured from either (a member mutation, or a mutating call on the bare name).

### `eslint-plugin-vue-no-ref-as-operand`

Ports eslint-plugin-vue 10.11.1 `vue/no-ref-as-operand` (default options): an identifier that holds a value wrapped by `ref`/`computed`/`toRef`/`customRef`/`shallowRef` imported from `vue`, `@vue/composition-api`, `@nuxtjs/composition-api` or `#imports` (or by the `defineModel` macro, or an alias or `toRefs` destructuring of one), used as an operand: `if`/`switch` test, unary, update, binary, assignment (not the right of `=`), conditional test, template literal (untagged), left of a `const` logical operator, or member object (not `.value`/`.effect`), plus ref arguments of an emit call (`defineEmits` result, `setup(props, { emit })`, `ctx.emit`) whose first argument is a string. Reads the script-block program (`ast.embedded`) outside the statement block whose host is the `<template>` element (template expressions are not visited by upstream), import resolution and variable bindings. Reported at the identifier.

### `eslint-plugin-vue-no-setup-props-reactivity-loss`

Ports eslint-plugin-vue 10.11.1 vue/no-setup-props-reactivity-loss (no options). Props containers are the first parameter of a `setup` function of a Vue component object (export default, Vue.component/mixin/extend, defineComponent, createApp, defineNuxtComponent, new Vue) and the variable(s) `const x = defineProps(...)`/`withDefaults(defineProps(...), ...)` binds at the top level of `<script setup>`. Reports a destructuring parameter / array or nested object pattern of the macro (destructuring), and, only for references in the root scope of the setup function or of `<script setup>` (no nested function, block, loop, class or catch between), a call holding a props member access, an array/object variable initializer or assignment holding one, a variable initializer or assignment whose member-chain base is a props member access or a plain destructured prop, a first template-literal/conditional part holding one, and a destructuring declarator/assignment of a props container (getProperty). Reads syntax and the file's scope table (binding definition names).

### `eslint-plugin-vue-no-side-effects-in-computed-properties`

Ports eslint-plugin-vue 10.11.1 `vue/no-side-effects-in-computed-properties` (default options). In an Options API getter (files without `<script setup>`) reports `this`/`const vm = this` member assignments, updates, deletes, calls of push/pop/shift/unshift/reverse/splice/sort/copyWithin/fill on a member of `this`, `Object.assign(this.x, ...)`, and `this.$set`/`Vue.set` (at the property). In a composition getter reports the same on a binding declared once, outside the getter, in `<script setup>` or the body of a Vue object's `setup` function, not imported (aliases are not followed, as upstream).

### `eslint-plugin-vue-no-template-shadow`

Ports eslint-plugin-vue 10.11.1 `vue/no-template-shadow` with default options (no allow list): a template variable (v-for alias or slot parameter, the parameters of the v-for / slot arrows of the script-block program, `ast.embedded`, inside the statement block whose host is the `<template>` element) whose name is already an outer template variable (a parameter of an enclosing view arrow), a top-level binding of `<script setup>`, a `defineProps` prop name (runtime array or object keys, `defineProps<{...}>()` type-literal properties, a local interface or type alias named as the type argument), or a static key of the Options API groups props, computed, data, asyncData, methods and setup (array strings, object members, and the object a data/asyncData/setup function returns) of `export default {}`, `defineComponent({})`, `Vue.extend/component/mixin(...)` or `new Vue({})`. Names compare as text. Reported at the variable identifier.

### `eslint-plugin-vue-no-unused-refs`

Ports eslint-plugin-vue 10.11.1 vue/no-unused-refs (no options): a static template `ref="name"` attribute is reported at its value literal (quotes included) unless the file uses the name, or has an unknown use of $refs. Used: `$refs.name`/`$refs['name']`/object-pattern destructuring of `$refs` (script `this.$refs` and bare `$refs`, template bare `$refs`), `useTemplateRef('name')`, a name `setup()` of a Vue object (export default, defineComponent/Vue.component/..., new Vue) returns, and, when the file has a <script setup>, any module/global binding of the program. Unknown use (`const a = $refs`, `a = $refs`, `f($refs)`, `return $refs`, `for (k in $refs)`, dynamic key or rest/computed destructuring) suppresses every report in the file. A clause over the file's own tree: the uses are read through `embedded` from the document in the script-block program (syntax, scope binds), each name compared with the attribute value through `host: { compare }`.

### `eslint-plugin-vue-no-unused-vars`

Ports eslint-plugin-vue 10.11.1 `vue/no-unused-vars` with default options (no ignorePattern): a template variable that nothing reads: a v-for alias or a slot parameter (`v-slot`, `#name`, Vue 2 `slot-scope`/`scope`). Reads the script-block program (`ast.embedded`): the parameters of the arrow that stands for a v-for (`expression_statement > sequence_expression > arrow_function`) or a slot (`expression_statement > arrow_function`) inside the template block, resolved by the engine's per-binding occurrences (a reference that is neither the definition nor the bare `Tag;` read the view writes for a component tag, which vue-eslint-parser does not make a reference). A Vue 2 filter's name is no reference either: vue-eslint-parser 10.4.1 splits an interpolation's or a `v-bind`'s expression at its top-level `|` into the expression and its filters (`{{ value | x }}`, `:a="v | f(y)"`) and references a filter's arguments, not its name (filter-callee: the right operand, or the callee of a call that is the right operand, of a chain of `|` that is the whole parenthesized statement whose host is an interpolation or a `v-bind`). A variable that is unreferenced is still exempt when a descendant element's raw tag name is the variable's (upstream's isUsedAsComponentTag: through `host`, the start tag of any element below the one whose `v-for` or slot directive declares the variable, a native tag included, compared by its tag name; a dotted tag's raw name is never an identifier). Parameters before a referenced one in the same arrow are exempt (after-used) unless destructured (an object-pattern property or an array-pattern element; a rest element of an object pattern is not). A v-for alias that a v-if/v-else-if condition of the same element reads (the v-for statement's host element holds that directive) is used (upstream resolves that condition in the element's scope; Vue 3 compiles it outside the v-for). Reported at the parameter identifier.

### `eslint-plugin-vue-no-use-v-if-with-v-for`

Ports eslint-plugin-vue 10.11.1 `vue/no-use-v-if-with-v-for` with default options (allowUsingIterationVar: false): a `v-if` directive on an element that also has `v-for`, reported at the `v-if` attribute. If the condition reads a name the element's `v-for` declares (a reference whose text equals a parameter name of the v-for arrow, as upstream compares names of `references` and `variables`) the message names the iterated source (`<source>` variable when it is an identifier, else expression); otherwise it asks to move the `v-if` to a wrapper. A clause over the file's own tree that reaches the script-block program through `embedded`: the `v-if` of an element with a `v-for` is `if (C) { (SRC), (ALIAS) => { <element> } }`, so a read of C reached from the `v-if` directive is compared with the parameters of the arrow its consequence holds, and the source expression is reached from the `v-for` directive.

### `eslint-plugin-vue-no-v-html`

Ports eslint-plugin-vue 10.11.1 `vue/no-v-html` with default options (no ignorePattern): every `v-html` directive attribute, with or without an argument, modifiers or value; reads the directive_name token. Reported at the directive attribute.

### `eslint-plugin-vue-no-v-text-v-html-on-component`

Ports eslint-plugin-vue 10.11.1 `vue/no-v-text-v-html-on-component` with default options (allow: [], ignoreElementNamespaces: false): a `v-text`/`v-html` directive (any argument or modifiers) on a component tag inside the template, reported at the directive attribute. A tag is a component as eslint-plugin-vue utils.isCustomComponent decides: a static `is` attribute, `:is`/`v-bind:is`/`.is` or `v-is` (case-insensitive), or its raw tag name (case-sensitive) is not in the well-known list of its namespace (HTML, SVG under `<svg>`, MathML under `<math>`; namespace propagation follows vue-eslint-parser: foreignObject/desc/title and MathML text integration points switch children back to HTML); the element lists are the shared regexes in the `custom-tag` utility.

### `eslint-plugin-vue-no-watch-after-await`

Ports eslint-plugin-vue 10.11.1 `vue/no-watch-after-await` (default options): a call of `watch` or `watchEffect` (not `watchPostEffect`/`watchSyncEffect`) imported from `vue`, `@vue/composition-api`, `@nuxtjs/composition-api` or `#imports` (named or namespace import), made directly in the body of a Vue object's `setup` function (export default, component/mixin/extend/defineComponent/createApp definition, `new Vue`; not `<script setup>`), after an `await` expression of that same function in source order, whose result is not used as a stop handle (declarator, assignment, call argument or callee, object property value, array element). Reported at the call.

### `eslint-plugin-vue-require-v-for-key`

Ports eslint-plugin-vue 10.11.1 `vue/require-v-for-key` (no options): an element that a `v-for` governs and that has no `v-bind:key`/`:key`/`.key` directive, reported at its start tag. An element is governed by its own `v-for`, or, when it is a child of a `<template>` or `<slot>` without a key, by that element's governors (checkKey recurses through keyless `<template>`/`<slot>` children), each governing `v-for` reporting once (`each` over the `v-for` directive, as ESLint reports once per visited `v-for`). `<template>` and `<slot>` elements themselves are never reported, nor is a component (utils.isCustomComponent: a static `is`, `:is`/`v-bind:is`/`.is` or `v-is`, or a raw tag name that is not a well-known name of its namespace, the `custom-tag` family shared with `no-v-text-v-html-on-component`). The `v-for` on the SFC's root `<template>` is never visited by ESLint and governs nothing. Reads the Vue tree only.

### `eslint-plugin-vue-require-valid-default-prop`

Ports eslint-plugin-vue 10.11.1 `vue/require-valid-default-prop` with default options: a prop's default must match the native constructors (String, Number, Boolean, Function, Object, Array, Symbol, BigInt) its `type` names; Object, Array and Function values must come from a factory, whose returns are checked too. Reads Options-API component objects (export default outside script setup, Vue.component/mixin/extend, component/createApp/defineComponent/defineNuxtComponent calls, new Vue, `// @vue/component` marks), top-level script-setup defineProps({...}) objects with withDefaults defaults and destructured defaults, and defineModel options. Props typed only by a TypeScript type argument (defineProps<T>(), defineModel<T>()) and a defineProps argument held in a const are not read.

### `eslint-plugin-vue-return-in-computed-property`

Ports eslint-plugin-vue 10.11.1 `vue/return-in-computed-property` (default options, treatUndefinedAsUnspecified: true). Reports a computed getter that can fall off its end or whose last `return` has no value: an Options API getter (`foo() {}`, `foo: function () {}`, `foo: { get() {} }`, `foo: { get: function () {} }`) of the `computed` object of a Vue object (`export default` outside `<script setup>`, `Vue.component`/`mixin`/`extend`, `app.component`/`mixin`, `defineComponent`, `createApp`, `defineNuxtComponent`, `new Vue`, or an object on the line after a `@vue/component` comment), and a composition getter given to `computed(...)` imported from `vue`, `@vue/composition-api`, `@nuxtjs/composition-api` or `#imports` (a function, or `{ get }`). Reads ESLint's code-path end reachability as the recursive `nc` utility (return, throw, break, continue, blocks, if/else, switch with a default, try/catch/finally, labelled statements, loops with a constant truthy test) and the last `return` statement in source order. Reported at the function (a method at its parameters).

### `eslint-plugin-vue-v-on-event-hyphenation`

Ports eslint-plugin-vue 10.11.1 `vue/v-on-event-hyphenation` with default options (always, ignore: [], ignoreTags: []): a static v-on argument (`@arg`, `v-on:arg`) on a component tag that contains an upper-case letter (value.toLowerCase() !== value) is reported at the directive attribute, with the whole key (modifiers included) in the message. The tree-sitter grammar splits `v-on:a:B` into `v-on:a` and `:B`; the `split-long-form` clause reads that case from the tag text. A tag is a component as eslint-plugin-vue utils.isCustomComponent decides: a static `is` attribute, `:is`/`v-bind:is`/`.is` or `v-is` (case-insensitive), or its raw tag name (case-sensitive) is not in the well-known list of its namespace (HTML, SVG under `<svg>`, MathML under `<math>`; namespace propagation follows vue-eslint-parser: foreignObject/desc/title and MathML text integration points switch children back to HTML); the element lists are the shared regexes in the `custom-tag` utility.

### `eslint-plugin-vue-valid-v-for`

Ports eslint-plugin-vue 10.11.1 `vue/valid-v-for` with default options (allowEmptyAlias: false). Syntax checks on every `v-for` attribute inside the template (the SFC's root `<template>` itself is never visited): an argument (reported at it), modifiers (reported from the first modifier's name to the last one's end, as upstream's loc), a missing or empty value (reported at the directive), and the alias list of `(value, key, index) in source` read from the attribute text like vue-eslint-parser's `for (let [aliases] in source)` parse: an empty value, key or index position (reported at the VForExpression: the value's text from the alias list's `(` to the end of the source, blanks and quotes left out) and a key or index that is not a plain identifier (a pattern, default or rest; reported at it). Key checks, per checkKey from each `v-for` (own element, or through keyless `<template>` children whose own `v-for` source does not read a name the outer `v-for` declares): a custom component (the `custom-tag` family shared with `no-v-text-v-html-on-component`) without a key is reported at its start tag, and a `v-bind:key` that reads none of the `v-for`'s alias names at the key directive. The name comparisons reach the script-block program through `embedded` from the inner `v-for` or the key directive: a read there is compared with a parameter of the v-for arrow `(SRC), (ALIAS) => { <element> }` whose host is the owning element's `v-for` directive (`host`, node identity with the owner); names compare as text, as upstream compares `references` to `variables` by name. Unreachable upstream: `unexpectedExpression` (vue-eslint-parser yields no expression for a value it cannot parse as `<alias> in <expression>`).
