# Review: vue-02

Branch reviewed: `tbelanger/pack-translations-vue-02` at `e365eefd88`. I read the five `.rerail` files, the library
they include (`vue/vue-02`, with `vue/names`, `vue/view` and `vue/compare` as used) and their JSON originals, then the
plugin's rule sources, docs and tests at tag `v10.11.1` (`lib/rules/*`, `lib/utils/index.js`, `lib/utils/ts-utils`),
before the author's NOTES and result file. The tool ran on a fleet box: ESLint 9.39.5, eslint-plugin-vue 10.11.1,
vue-eslint-parser 10.4.2 (`espree` 10.4.0 for JavaScript, `@typescript-eslint/parser` 8.74.0 with TypeScript 6.0.3 for
`lang="ts"`), flat config, the five rules at their defaults. Each translation ran with `rerail-engine run-policies` and
this branch's `docs/rule-syntax/lib`. Every counterexample below is a whole file. Lines are ESLint's and the engine's;
run-policies gives no column, so a column is named only where it decides which node is meant.

The plugin's own tests agree first: every case of the five `tests/lib/rules/*.test.ts` files that has no options and
a `.vue` filename (155 cases) gives the same file:line set from ESLint and from the translation. The defects below are
all outside those tests.

| Rule | Verdict |
|---|---|
| `eslint-plugin-vue-no-setup-props-reactivity-loss` | 5 defects (switch discriminant, `for` without a lexical declaration, `export default {} as/satisfies`, `// @vue/component`, `defineComponent` from `#imports`) |
| `eslint-plugin-vue-no-side-effects-in-computed-properties` | 1 defect (template-literal mutator name), 1 message difference |
| `eslint-plugin-vue-no-template-shadow` | 8 defects (plain `<script>` bindings, TypeScript class/enum/type/interface names, `defineProps` intersections and alias chains, `defineProps(const)`, computed and template-literal keys, `// @vue/component`, non-last object argument, same-element v-slot before v-for) |
| `eslint-plugin-vue-no-unused-refs` | 5 defects (enum names, computed keys of setup()'s return, `// @vue/component`, `useTemplateRef` in the template, commented or parenthesised `useTemplateRef` argument) |
| `eslint-plugin-vue-no-unused-vars` | 2 defects (attributes of the slot-declaring element, a v-if arrow parameter that shadows the alias) |

Three of the defects are one gap in three rules: none of them takes an object under a `// @vue/component` line as the
component (the plugin's `getVueObjectType` returns `"mark"` for it). no-side-effects-in-computed-properties has that
clause (`vue_object`'s fourth rule) and could lend it.

## eslint-plugin-vue-no-setup-props-reactivity-loss

Agreed: the plugin's 44 test cases; braceless `if`/`while`/labelled statements in setup()'s own code; `try` and
`catch` blocks; class fields and `class extends mix(props.d)`; `new Map(props.a)`, `props.a + 1`, `props.a || {}` (none
reported); destructured defineProps names read as `x = a`, `a.length`, `rest.x`, `watch(a)` and `` `${c}` ``.

The plugin keeps the `props` references "from" setup()'s (or the module's) own scope (`reference.from ===
currentScope`), so what decides is eslint-scope's scopes, not syntax. The translation's `barrier` approximates them
by node kind; defects 1 and 2 are where the two part.

**Defect 1 (miss): a call in a `switch` discriminant.** eslint-scope visits the discriminant before it opens the switch
scope, so `props.size` there is read from setup()'s scope and the call holding it is reported. `barrier` makes the
whole `switch_statement` a barrier.

```vue
<!-- p01-switch.vue -->
<script>
export default {
  setup(props) {
    switch (String(props.size)) {
      case 'a': break
    }
    switch (props.kind) {
      default: break
    }
  }
}
</script>
```

- ESLint: `p01-switch.vue:4` (line 7 is not reported: no call, declaration or assignment holds the read).
- translation: nothing.

```vue
<!-- p02-switch-script.vue -->
<script setup>
const props = defineProps({ size: String })
switch (normalize(props.size)) {
  default: break
}
</script>
```

- ESLint: `p02-switch-script.vue:3`.
- translation: nothing.

**Defect 2 (miss): a `for` statement that declares nothing lexically.** eslint-scope opens a `for` scope only for a
`let`/`const` head. Under `for (i = 0; …)` or `for (var x of …)` the head and a braceless body stay in setup()'s scope;
`barrier` makes every `for_statement` and `for_in_statement` a barrier.

```vue
<!-- p03-for-novar.vue -->
<script>
export default {
  setup(props) {
    let i
    for (i = 0; i < 3; i++) track(props.a)
    for (var x of toList(props.list)) {}
    for (const y of toList(props.list)) {}
  }
}
</script>
```

- ESLint: `p03-for-novar.vue:5`, `:6` (line 7's `const` head opens a scope: not reported).
- translation: nothing.

**Defect 3 (extra): `export default { … } as X` and `satisfies X`.** `getVueObjectType` skips the `as` to reach the
`ExportDefaultDeclaration`, then requires `isVueComponentFile`, whose `node.declaration.type === "ObjectExpression"`
fails for a `TSAsExpression`; `satisfies` is not skipped at all. Neither object is a component, so its setup() is not
checked. `vue_object`'s second clause takes both.

```vue
<!-- p04-export-as.vue -->
<script lang="ts">
export default {
  setup(props: any) {
    const a = props.a
  }
} as any
</script>
```

```vue
<!-- p05-export-satisfies.vue -->
<script lang="ts">
import type { Component } from 'vue'
export default {
  setup(props: any) {
    const a = props.a
  }
} satisfies Component
</script>
```

- ESLint: nothing.
- translation: `p04-export-as.vue:4`, `p05-export-satisfies.vue:5`.

**Defect 4 (miss): an object under `// @vue/component`.**

```vue
<!-- p06-vue-component-comment.vue -->
<script>
// @vue/component
const Comp = {
  setup(props) {
    const a = props.a
  }
}
export default Comp
</script>
```

- ESLint: `p06-vue-component-comment.vue:5`.
- translation: nothing.

**Defect 5 (miss): `defineComponent` imported from `#imports`.** `isGlobalOrImportedFromVue` first runs a
ReferenceTracker over `createCompositionApiTraceMap`, which includes `#imports`; only its fallback checks the
`vue`/`@vue/composition-api`/`@nuxtjs/composition-api` list. `own_binding`'s regex has no `#imports`, so the import is
an own binding and `defineComponent` no definer.

```vue
<!-- p07-imports.vue -->
<script>
import { defineComponent } from '#imports'
export default defineComponent({
  setup(props) {
    const a = props.a
  }
})
</script>
```

- ESLint: `p07-imports.vue:5`.
- translation: nothing.

## eslint-plugin-vue-no-side-effects-in-computed-properties

Agreed: the plugin's 22 test cases; `this.$set` passed as an argument; `||=` and `**=`; a parameter of a function
nested in setup() and a class name mutated in computed(); `Object.assign(state, …)` (not reported) and
`Object.assign(state.inner, …)`; `app.component('x', { computed })`; `computed({ get: () => … })` and
`computed({ get() {} })`; options API computed in a file with `<script setup>` (not checked) next to that file's
computed(); arrow `get:` (not a getter), `'c'()`; `Vue.set` and `this.$set` in a `// @vue/component` object;
`computed<number>(…)`, `(): T =>` getters, `(x as T).push()` (not reported); `setup: () => {}`; a declaration in a
function or block nested in setup(), and one in a method (not reported). An undeclared, auto-imported `computed` is
not reported by either: `iterateGlobalReferences` follows only configured globals.

**Defect 1 (miss): a mutator named by a template literal.** `findMutating` takes the name with
`getStaticPropertyName`, which reads `` [`push`] ``. `mutator_callee`'s subscript clause wants a `string`.

```vue
<!-- e07-template-literal-mutator.vue -->
<script setup>
import { computed, ref } from 'vue'
const list = ref([])
const c = computed(() => list.value[`push`](1))
const d = computed(() => list.value['reverse']())
</script>
```

- ESLint: `e07-template-literal-mutator.vue:4`, `:5`.
- translation: `:5`.

**Message: a template-literal computed key.** `` [`d`]() {} `` in `computed`: the plugin names it `"d"`
(`getStaticPropertyName`); `computed_name` handles quotes only and gives `"Unknown"`. Same line (`e11:7`), so only the
`{x}` parameter differs.

## eslint-plugin-vue-no-template-shadow

Agreed: the plugin's 25 test cases; `import type` names, function declarations and namespaces in `<script setup>`; a
`for (const … of)` binding (not module scope); `export default { … } as any` (not a component for either); a `data()`
with a nested `return` (only the top-level one counts), spreads of `mapActions`, a `get`/`set` computed pair; `<script
setup lang="ts" generic="T">` (`T` not reported by either).

**Defect 1 (miss): top-level bindings of the plain `<script>` in a file with `<script setup>`.** vue-eslint-parser
puts both scripts in one program, so the plugin's module-scope walk (`isScriptSetup` branch) sees the plain script's
imports and declarations too. `declared`'s first clause requires `vv_setup_stmt`.

```vue
<!-- s01-plain-script.vue -->
<script>
import { item } from './items'
const row = 1
export default {}
</script>
<script setup>
const other = 1
</script>
<template>
  <div v-for="item in items">{{ item }}</div>
  <div v-for="row in items">{{ row }}</div>
</template>
```

- ESLint: `s01-plain-script.vue:10`, `:11`.
- translation: nothing.

**Defect 2 (miss): TypeScript class, enum, type alias and interface names in `<script setup lang="ts">`.**
typescript-eslint's scope manager gives each a module-scope variable with a definition. `declared` keeps only
`identifier` and `shorthand_property_identifier_pattern` names, and a TypeScript class, type alias or interface name
is a `type_identifier`. An enum's name is missed too, though it is an `identifier`: it is not in `vv_module_name`
(no-unused-refs defect 1 shows the same from the other side).

```vue
<!-- s16-types.vue -->
<script setup lang="ts">
type item = string
interface row { a: 1 }
class col {}
</script>
<template>
  <div v-for="item in list">{{ item }}</div>
  <div v-for="row in list">{{ row }}</div>
  <div v-for="col in list">{{ col }}</div>
</template>
```

- ESLint: `s16-types.vue:7`, `:8`, `:9`.
- translation: nothing.

```vue
<!-- s09-ts-decls.vue -->
<script setup lang="ts">
import type { item } from './t'
enum row { A }
function cell() {}
class col {}
namespace ns { export const x = 1 }
for (const loopv of [1]) {}
</script>
<template>
  <div v-for="item in list">{{ item }}</div>
  <div v-for="row in list">{{ row }}</div>
  <div v-for="cell in list">{{ cell }}</div>
  <div v-for="col in list">{{ col }}</div>
  <div v-for="ns in list">{{ ns }}</div>
  <div v-for="loopv in list">{{ loopv }}</div>
</template>
```

- ESLint: `s09-ts-decls.vue:10`, `:11`, `:12`, `:13`, `:14`.
- translation: `:10`, `:12`, `:14` (misses the enum on `:11` and the class on `:13`). In plain JavaScript a class
  name is an `identifier` and agrees (`s15-class-js.vue`).

**Defect 3 (miss): `defineProps` type arguments that are an intersection or union, or an alias of an alias.**
`flattenTypeNodes` (ts-utils) splits `A & B` and `A | B` and follows a type alias to its annotation, recursively.
`declared`'s type clauses take an `object_type` right under the type arguments, or an interface or alias body named
there.

```vue
<!-- s02-intersection.vue -->
<script setup lang="ts">
defineProps<{ a: string } & { item: string }>()
</script>
<template>
  <div v-for="item in list">{{ item }}</div>
</template>
```

```vue
<!-- s03-alias-chain.vue -->
<script setup lang="ts">
type Base = { item: string }
type Props = Base
defineProps<Props>()
</script>
<template>
  <div v-for="item in list">{{ item }}</div>
</template>
```

- ESLint: `s02-intersection.vue:5`, `s03-alias-chain.vue:7`.
- translation: nothing.

**Defect 4 (miss): `defineProps(name)` of a `const` object.** `getComponentPropsFromDefineProps` resolves the argument
with `getObjectOrArray`, which follows an identifier to a single `const` declarator's object or array.

```vue
<!-- s04-props-const.vue -->
<script setup>
const propsDef = { item: String }
defineProps(propsDef)
</script>
<template>
  <div v-for="item in list">{{ item }}</div>
</template>
```

- ESLint: `s04-props-const.vue:6`.
- translation: nothing.

**Defect 5 (miss): a computed group key, and template literals as names.** `iterateProperties` names a group with
`getStaticPropertyName`, which reads `['data']`; `group_name` matches the key's raw text, which `['data']` is not.
`iterateArrayExpression` takes a template literal as a prop name, and `getStaticPropertyName` reads a
`` [`row`] `` key; `declared` and `member_key` take strings only.

```vue
<!-- s06-computed-group-key.vue -->
<script>
export default {
  ['data']() { return { item: 1 } },
  'methods': { row() {} }
}
</script>
<template>
  <div v-for="item in list">{{ item }}</div>
  <div v-for="row in list">{{ row }}</div>
</template>
```

- ESLint: `s06-computed-group-key.vue:8`, `:9`.
- translation: `:9`.

```vue
<!-- s13-template-literal.vue -->
<script>
export default {
  props: [`item`],
  data() { return { [`row`]: 1 } }
}
</script>
<template>
  <div v-for="item in list">{{ item }}</div>
  <div v-for="row in list">{{ row }}</div>
</template>
```

- ESLint: `s13-template-literal.vue:8`, `:9`.
- translation: nothing.

**Defect 6 (miss): an object under `// @vue/component`.** `component_object` has no clause for it.

```vue
<!-- s07-vue-component-comment.vue -->
<script>
// @vue/component
const Comp = {
  data() { return { item: 1 } }
}
export default Comp
</script>
<template>
  <div v-for="item in list">{{ item }}</div>
</template>
```

- ESLint: `s07-vue-component-comment.vue:9`.
- translation: nothing.

**Defect 7 (extra): an object that is not a definer's last argument.** `getVueObjectType` takes only
`parent.arguments.at(-1)`; `component_object` takes any argument of the call.

```vue
<!-- s12-last-arg.vue -->
<script>
import Vue from 'vue'
Vue.component('x', { data() { return { item: 1 } } })
Vue.mixin({ methods: { row() {} } }, 1)
</script>
<template>
  <div v-for="item in list">{{ item }}</div>
  <div v-for="row in list">{{ row }}</div>
</template>
```

- ESLint: `s12-last-arg.vue:7`.
- translation: `:7`, `:8`.

**Defect 8 (wrong node): a `v-slot` before a `v-for` on one element, with the same name.** The plugin walks
`node.variables` in attribute order: the slot's `row` is pushed first, and the v-for's `row` is the one reported.
`taken`'s first clause reports the slot's parameter, whose arrow the view nests inside the v-for's.

```vue
<!-- s14-same-element-lines.vue -->
<template>
  <div>
    <Comp
      v-slot="row"
      v-for="row in rows">{{ row }}</Comp>
  </div>
</template>
```

- ESLint: `s14-same-element-lines.vue:5` (column 14, the v-for alias).
- translation: `:4` (the slot parameter).

With `v-for` first (`<Comp v-for="item in items" v-slot="item">`) both report the slot's `item`.

## eslint-plugin-vue-no-unused-refs

Agreed: the plugin's 26 test cases; `this.$refs.box` inside the template (a property, so no read for either); `ref=""`
(reported as `''` by both); `v-for="r in $refs"` (no read); `this.saved = this.$refs`, `[this.$refs]`,
`{ r: this.$refs }`, `for (… of this.$refs)` (none escapes); `(this.$refs as any).box` (no read) and
`(this.$refs).panel` (read); `this.$refs?.box?.focus()`, `const { cell = null } = this.$refs`; `import type` and
`import { type … }` in `<script setup>`; plain-`<script>` bindings in a file with `<script setup>`; a TypeScript or
JavaScript class name; `defineComponent` from a non-Vue module.

**Defect 1 (extra): an enum name in `<script setup lang="ts">`.** The plugin counts every module-scope variable with a
definition, an enum's included. `vv_module_name` has no enum clause.

```vue
<!-- r01-class-enum.vue -->
<script setup lang="ts">
class box {}
enum panel { A }
</script>
<template>
  <div ref="box"></div>
  <div ref="panel"></div>
</template>
```

- ESLint: nothing.
- translation: `r01-class-enum.vue:7`.

**Defect 2 (extra): computed keys of the object setup() returns.** `iterateObjectExpression` names a `Literal` or
`TemplateLiteral` key with `getStaticPropertyName`, computed or not. `ref_name`'s setup clauses take a
`property_identifier`, `string` or `number` key only.

```vue
<!-- r03-setup-keys.vue -->
<script>
export default {
  setup() {
    return { ['box']: 1, [`panel`]: 2 }
  }
}
</script>
<template>
  <div ref="box"></div>
  <div ref="panel"></div>
</template>
```

- ESLint: nothing.
- translation: `r03-setup-keys.vue:9`, `:10`.

**Defect 3 (extra): setup() of an object under `// @vue/component`.** `defineVueVisitor` calls `onVueObjectEnter` for
it; `vue_object` has no clause for it.

```vue
<!-- r04-vue-component-comment.vue -->
<script>
// @vue/component
const Comp = {
  setup() { return { box: 1 } }
}
export default Comp
</script>
<template>
  <div ref="box"></div>
</template>
```

- ESLint: nothing.
- translation: `r04-vue-component-comment.vue:9`.

**Defect 4 (miss): `useTemplateRef("x")` in a template expression.** The plugin's `CallExpression` visitor is a script
visitor; template expressions reach only `VExpressionContainer`, which looks for `$refs`. `read`'s first clause takes
`ref_name` from every guest of the document, the template's view included.

```vue
<!-- r05-template-this.vue -->
<template>
  <div ref="box" @click="this.$refs.box.focus()"></div>
  <div ref="panel" @click="useTemplateRef('panel')"></div>
</template>
```

- ESLint: `r05-template-this.vue:2`, `:3`.
- translation: `:2`.

**Defect 5 (extra): a `useTemplateRef` argument after a comment, or in parentheses.** The plugin reads
`arguments[0]`, which neither the comment nor ESTree's dropped parentheses change. `first_arg` wants `(` and spaces
only before the argument, and the parenthesised string is not a direct child of `arguments`.

```vue
<!-- r13-paren-arg.vue -->
<script setup>
import { useTemplateRef } from 'vue'
useTemplateRef(/* el */ 'box')
useTemplateRef(('panel'))
</script>
<template>
  <div ref="box"></div>
  <div ref="panel"></div>
</template>
```

- ESLint: nothing.
- translation: `r13-paren-arg.vue:7`, `:8`.

## eslint-plugin-vue-no-unused-vars

Agreed: the plugin's 38 option-free test cases; an element named like its own variable (`<div v-for="div in xs">`,
reported: only descendants count); a used variable of a nested or a later sibling v-for does not count as "after";
a v-for and a slot on one element are separate groups; writes (`@click="item = 1"`, `v-model`); Vue 2 filter names
(`{{ value | f }}`, not a read); a variable shadowed by an arrow parameter in a `{{ }}`; `v-if` on a v-for element
reading the alias or a later parameter; a dotted tag, a lower-case tag for an upper-case name, a tag inside a nested
`<template>`.

**Defect 1 (extra): the other attributes of the element that declares a slot scope.** vue-eslint-parser resolves an
element's variables over every attribute of that element, so `v-if="a"` or `:prop="a"` on the `v-slot`/`slot-scope`
element reads `a`. The view puts those attributes outside the slot's arrow, and `if_read` covers only a v-for's
`v-if` (`child(sq, a, _), kind(sq, "sequence_expression")`).

```vue
<!-- u03-slot-vif.vue -->
<template>
  <div>
    <Comp v-slot="{ a }" v-if="a">x</Comp>
    <Comp><template #default="{ b }" v-if="b">x</template></Comp>
  </div>
</template>
```

```vue
<!-- u07-slot-attr.vue -->
<template>
  <div>
    <Comp v-slot="{ a }" :prop="a"></Comp>
    <Comp slot-scope="{ b }" :prop="b"></Comp>
  </div>
</template>
```

- ESLint: nothing.
- translation: `u03-slot-vif.vue:3`, `:4`; `u07-slot-attr.vue:3`, `:4`.

**Defect 2 (miss): a `v-if` whose arrow parameter shadows the alias.** `if_read` matches any identifier in the
condition by text; the plugin counts only references that resolve to the alias.

```vue
<!-- u04-vif-arrow.vue -->
<template>
  <ul>
    <li v-for="x in xs" v-if="ys.some(x => x)">y</li>
    <li v-for="z in xs" v-if="obj.z">y</li>
  </ul>
</template>
```

- ESLint: `u04-vif-arrow.vue:3`, `:4`.
- translation: `:4`.

## The author's result file

`vue-02.result.json` marks all five rules `same`, so it has no `different` entry whose `right` side needs checking.
The verdicts hold: `ptcheck --box pt-w01 --ref tbelanger/pack-translations-vue-02 -- --pack vue --rules <the five>`
at `e365eefd88` reports 5 same, 0 different (fixtures 773 files, samples-vue 141), with no original-only or
translation-only finding on either corpus.

The same run fails the pack's gate on five problems, one per rule: "reports the same findings as its original now;
remove it from different". `docs/rule-syntax/packs/vue/equivalence.json` still lists all five under `different`, with
`"right": "unknown"` and the counts of the earlier versions (reactivity-loss 61 + 12 original-only, side-effects 51,
…). Those entries are stale and should go; there is no `right` side in them to check against ESLint.

`same` is against the JSON originals. The defects above are against ESLint; the corpora do not contain their shapes,
so the suite cannot tell whether the originals share them.
