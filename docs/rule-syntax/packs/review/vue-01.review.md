# Review: vue-01

Branch reviewed: `tbelanger/pack-translations-vue-01` at `deff5c564c`. I read the five `.rerail` files, the library
they include (`vue/vue-01`, `vue/component`, `vue/mutation`, `tsx/eslint-a-common`, `tsx/names`) and their JSON
originals, then the plugin's source, docs and tests at tag `v10.11.1` (`lib/rules/*`, `lib/utils/index.js`,
`lib/utils/ref-object-references.ts`), before the author's NOTES and result file. The tool ran on a fleet box:
ESLint 9.39.5, eslint-plugin-vue 10.11.1, vue-eslint-parser 10.4.2 (`espree` 10.4.0 for JavaScript,
`@typescript-eslint/parser` 8.74.0 with TypeScript 6.0.3 for `lang="ts"`), flat config, the five rules at their
defaults. Each translation ran with `rerail-engine run-policies` and this branch's `docs/rule-syntax/lib`. Every
counterexample below is a whole file. Lines are ESLint's and the engine's.

| Rule | Verdict |
|---|---|
| `eslint-plugin-vue-no-async-in-computed-properties` | 6 defects (arrow-valued option getters, `get x()` getters, `const vm = this`, renamed `computed` import, an arrow whose body starts a line later, `ignoredObjectNames` through `!` and `as`) |
| `eslint-plugin-vue-no-dupe-keys` | 4 defects (option value under `as`, props read through a shorthand, type names, intersection props) |
| `eslint-plugin-vue-no-export-in-script-setup` | 1 defect (`export declare …`) |
| `eslint-plugin-vue-no-mutating-props` | 3 defects (type alias named like a prop, intersection props, `v-model:[arg]`) |
| `eslint-plugin-vue-no-ref-as-operand` | 7 defects (renamed `ref` import, template-literal event name, renamed and defaulted setup context, nested and rest `toRefs` patterns, `ref() as T`, `defineModel()` reassigned a ref) |

Some defects are in the shared library vue/component, not the rules: `vvue_import_call` (renamed imports),
`vprop_name` (intersection props) and `vgetter` (arrow-valued and `get x()` computed getters). Each is listed under
every rule it was confirmed on.

## eslint-plugin-vue-no-async-in-computed-properties

Agreed: `async foo()`, `await`, `new Promise`, `.then`, `Promise.all`, `setTimeout(f, 1)`, computed() in
`<script setup>` and in `defineComponent({ setup() {} })`, a file with both scripts (the Options API computed is then
not checked, the `<script setup>` computed() is), a `// @vue/component` object, `ignoredObjectNames: ["z"]` on
`z.number().catch(42)`.

**Defect 1 (extra): an Options API computed whose value is an arrow function.** `utils.getComputedProperties` takes a
computed's body only from a `FunctionExpression` value, or from a `get` property whose value is a `FunctionExpression`;
an arrow gives `value: null`, so nothing in it is checked. `vgetter` (vue/component) takes any `function_node`.

```vue
<!-- a1-arrow-options.vue -->
<script>
export default {
  computed: {
    foo: () => fetch('x').then(r => r),
    bar: { get: () => Promise.resolve(1) }
  }
}
</script>
```

- ESLint: nothing.
- translation: `a1-arrow-options.vue:4`, `:5`.

**Defect 2 (miss): a `get foo()` accessor in `computed`.** `getComputedProperties` reads every `Property` with a
`FunctionExpression` value, whatever its `kind`, so an accessor's body is the computed `foo`. `vgetter`'s method clause
requires `!op(p, "get")`.

```vue
<!-- a2-accessor.vue -->
<script>
export default {
  computed: {
    get foo() {
      return fetch('x').then(r => r)
    }
  }
}
</script>
```

- ESLint: `a2-accessor.vue:5` asynchronous action in "foo".
- translation: nothing.

**Defect 3 (miss): `$nextTick` on a `const` alias of `this`.** `isNextTick` uses `utils.isThis`, which also holds of an
identifier whose only definition is `const vm = this`. `next_tick` wants `kind(o, "this")`.

```vue
<!-- a3-this-alias.vue -->
<script>
export default {
  computed: {
    foo() {
      const vm = this
      vm.$nextTick(() => {})
      return 1
    }
  }
}
</script>
```

- ESLint: `a3-this-alias.vue:6`.
- translation: nothing.

**Defect 4 (miss, library): a renamed import of `computed`.** The rule finds computed() through ReferenceTracker's ESM
map, which follows the imported name. `vvue_import_call(c, x)` (vue/component) binds x to the callee's local text, so
`import { computed as c }` gives `x = "c"`. The fix is in the library: take `import_binding`'s fourth column (the
imported name) for a named import. The same defect is no-ref-as-operand's defect 1.

```vue
<!-- a4-renamed-computed.vue -->
<script setup>
import { computed as c } from 'vue'
const x = c(async () => 1)
const y = c(() => fetch('x').then(r => r))
</script>
```

- ESLint: `a4-renamed-computed.vue:3`, `:4`.
- translation: nothing.

**Defect 5 (extra): an async arrow whose body starts on a later line.** `verify` reports only when
`node.loc.start.line >= body.loc.start.line`. For `async` the node is the function itself, which starts before its
body, so the async report is dropped when the expression body starts on the next line (the `await` inside is still
reported). `report_at` has no such check.

```vue
<!-- a6-arrow-nextline.vue -->
<script setup>
import { computed } from 'vue'
const x = computed(async () =>
  await load()
)
</script>
```

- ESLint: `a6-arrow-nextline.vue:4` (await).
- translation: `:3`, `:4`.

**Defect 6 (extra, option): `ignoredObjectNames` through a TypeScript `!` or `as`.** `getRootObjectName` steps through
`skipWrapper`, which follows any `.expression` (`TSNonNullExpression`, `TSAsExpression`, `ChainExpression`), so
`z.a?.['b'].c!.d.method().catch(…)` and `(z as any).number().catch(…)` have the root `z`. `root_step` goes through
parentheses only, so these chains have no root and are reported. The first line is the plugin's own valid test
(`tests/lib/rules/no-async-in-computed-properties.test.ts`, the last valid case). `run-policies` takes no options, so
the translation ran as a scratch copy in /tmp on the box with `option_item("ignoredObjectNames", _, y)` replaced by a
local fact `opt_ign("z")`; nothing else changed.

```vue
<!-- a9-ignored.vue  (option {"ignoredObjectNames": ["z"]}) -->
<script setup lang="ts">
import { computed } from 'vue'
import { z } from 'zod'
const foo = computed(() => z.a?.['b'].c!.d.method().catch(err => err))
const bar = computed(() => (z as any).number().catch(42))
const baz = computed(() => z.number().catch(42))
</script>
```

- ESLint: nothing.
- translation: `a9-ignored.vue:4`, `:5`.

## eslint-plugin-vue-no-dupe-keys

Agreed: Options API duplicates across props, computed, methods and data (a method); a `// @vue/component` object;
`<script setup>` bindings named like props (a `const`, `let foo; foo = …`, an import, a function, a class, a hoisted
`var` in a block, an `import type`); not reported: the destructured prop, a renamed one, an initialiser that reads
`props.foo`.

**Defect 1 (extra): an option value under `as`.** `iteratePropertyValue` switches on the value's own type, with no
`skipTSAsExpression`, so `props: ['a'] as any` declares nothing. `vopt` reads the value through `vunwrap`.

```vue
<!-- d1-as-options.vue -->
<script lang="ts">
export default {
  props: ['a'] as any,
  data() {
    return { a: 1 }
  }
}
</script>
```

- ESLint: nothing.
- translation: `d1-as-options.vue:5`.

**Defect 2 (extra): an initialiser that reads the props variable through a shorthand property.** The rule skips a
binding whose initialiser contains any reference to the props variable; `{ props }` is one. `read_in` takes only
`identifier` nodes, and the shorthand is a `shorthand_property_identifier`.

```vue
<!-- d2-shorthand-props.vue -->
<script setup>
import { useX } from './x'
const props = defineProps(['modelValue'])
const modelValue = useX({ props })
</script>
```

- ESLint: nothing.
- translation: `d2-shorthand-props.vue:4`.

**Defect 3 (miss): a type alias or interface named like a prop.** `findVariable` looks the prop's name up in the scope
manager, and typescript-eslint's puts type aliases and interfaces in the module scope as variables with a definition.
`top_binding` reads `decl_site(…, "value")`, which has the `import type` but not a `type` or `interface` declaration.

```vue
<!-- d3-type-names.vue -->
<script setup lang="ts">
import type { foo } from './t'
type bar = string
const props = defineProps<{ foo: string; bar: string }>()
</script>
```

```vue
<!-- d13-interface-name.vue -->
<script setup lang="ts">
interface foo { x: string }
defineProps<{ foo: string }>()
</script>
```

- ESLint: `d3-type-names.vue:2`, `:3`; `d13-interface-name.vue:2`.
- translation: `d3-type-names.vue:2`; nothing on d13.

**Defect 4 (miss, library): props of an intersection type.** The rule's type-based props resolve
`type Props = Base & { bar: string }` to both members. `vprop_name` (vue/component) reads a type literal, or an
interface or type alias whose value is an `object_type`; an intersection gives no props. The same defect is
no-mutating-props' defect 2.

```vue
<!-- d12-intersection-dupe.vue -->
<script setup lang="ts">
interface Base { foo: string }
type Props = Base & { bar: string }
defineProps<Props>()
const foo = 1
const bar = 2
</script>
```

- ESLint: `d12-intersection-dupe.vue:5`, `:6`.
- translation: nothing.

(`interface Props extends Base` gives the plugin no inherited props either: both sides report nothing there.)

## eslint-plugin-vue-no-export-in-script-setup

Agreed: `export const`, `export {}`, `export enum`, `export { type T, type U } from …` (allowed), `export type * from …`
(allowed).

**Defect 1 (extra): `export declare …`.** typescript-estree gives an `ExportNamedDeclaration` `exportKind: "type"` when
its declaration is `declare`d (any kind: `const`, `function`, `class`, `namespace`), not only for type aliases and
interfaces, and the rule returns on `exportKind === 'type'`. `type_decl` accepts an `ambient_declaration` only around a
type alias or interface.

```vue
<!-- e1-declare.vue -->
<script setup lang="ts">
export declare const x: number
export declare function f(): void
export type * from './t'
export const a = 1
</script>
```

```vue
<!-- e2-ts-forms.vue -->
<script setup lang="ts">
export declare class C {}
export declare namespace N {}
export enum E { A }
export { type T, type U } from './t'
export {}
</script>
```

- ESLint: `e1-declare.vue:5`; `e2-ts-forms.vue:4`, `:6`.
- translation: `e1-declare.vue:2`, `:3`, `:5`; `e2-ts-forms.vue:2`, `:3`, `:4`, `:6`.

## eslint-plugin-vue-no-mutating-props

Agreed: script `this.x = …` and `const vm = this; vm.a.push(1)`, `setup: (props) => { props.a = 1 }`,
`setup({ list, obj: { deep } })` (`.push`, `.sort`, `Object.assign` and deep writes reported, `list = []` not),
`<script setup>` destructured props (`list.push`), the template's free names (`a++`, `a.push(2); b++`, an arrow handler
`() => { a = 1 }`), a `v-for` alias of the same name left alone, `v-model="list"`, `v-model="props.a"`,
`v-model="props[k]"`, `props.x.y = 1; props = {}`, the template's component: `<script setup>`'s defineProps over
`export default`, the last of two `Vue.component` objects, `const C = defineComponent(…); export default C`, a
`// @vue/component` object.

**Defect 1 (extra): a type alias named like a prop.** For `<script setup>` the template's props drop every name with a
definition in the module scope (`extractDefineVariableNames`); with typescript-eslint's scope manager that includes
`type foo`. `top_level_name` reads `decl_site`, which has no row for a type alias, so `foo` stays a prop.

```vue
<!-- m1-type-alias.vue -->
<template>
  <button @click="foo = 'x'">x</button>
</template>
<script setup lang="ts">
type foo = string
const props = defineProps<{ foo: string }>()
</script>
```

- ESLint: nothing from this rule (no-dupe-keys: `:5`, its defect 3).
- translation: `m1-type-alias.vue:2`.

**Defect 2 (miss, library): props of an intersection type.** As no-dupe-keys' defect 4: `vprop_name` gives no props
for `type Props = Base & { bar: string }`, so the template's writes are not checked.

```vue
<!-- d10-intersection.vue -->
<template>
  <button @click="foo = 'x'; bar = 'y'">x</button>
</template>
<script setup lang="ts">
interface Base { foo: string }
type Props = Base & { bar: string }
const props = defineProps<Props>()
</script>
```

- ESLint: `d10-intersection.vue:2` (foo, bar).
- translation: nothing.

**Defect 3 (miss, rare): a `v-model` dynamic argument.** The rule's v-model selector is
`VAttribute[…model…] VExpressionContainer > *`, which also matches the expression container of the directive's
dynamic argument, so a prop named there is reported. `model` reads the directive's value only.

```vue
<!-- m10-dyn-arg.vue -->
<template>
  <comp v-model:[a]="x" />
</template>
<script setup>
import { ref } from 'vue'
defineProps(['a'])
const x = ref(1)
</script>
```

- ESLint: `m10-dyn-arg.vue:2` "a".
- translation: nothing.

## eslint-plugin-vue-no-ref-as-operand

Agreed: `if`, `switch`, `?:` test, unary (`typeof`), `++`, binary, `x += n`, `n || 1` on a `const` (not on a `let`),
`${n}` (not tagged), `n.toFixed()`, `n?.toFixed()`, `x = n` not reported, a shadowing parameter, `const m = n` alias,
`let foo; foo = ref(5); let bar = foo; bar = 4`, `let k = ref(0); k = 4`, `toRefs` shorthand and renamed
destructuring, `emit('change', n)` from `defineEmits`, `setup(props, { emit })`.

**Defect 1 (miss, library): a renamed import of `ref`.** As no-async-in-computed-properties' defect 4:
`vvue_import_call` matches the local name, so `ref as r` is no maker.

```vue
<!-- r1-renamed-ref.vue -->
<script setup>
import { ref as r } from 'vue'
const n = r(0)
const m = n + 1
</script>
```

- ESLint: `r1-renamed-ref.vue:4`.
- translation: nothing.

**Defect 2 (miss): an event name written as a template literal.** `getNameParamNode` takes `utils.isStringLiteral`,
which includes a template literal without expressions. `emit_call` wants `kind(a, "string")`.

```vue
<!-- r2-template-literal-emit.vue -->
<script setup>
import { ref } from 'vue'
const emit = defineEmits(['change'])
const n = ref(0)
emit(`change`, n)
emit('change', n)
</script>
```

- ESLint: `r2-template-literal-emit.vue:5`, `:6`.
- translation: `:6`.

**Defect 3 (miss): a renamed `emit` from setup's context.** The rule takes `findAssignmentProperty(contextParam, 'emit')`
and follows its value, so `{ emit: e }` makes `e` the emitter. `emitter` takes a
`shorthand_property_identifier_pattern` only.

```vue
<!-- r3-emit-renamed.vue -->
<script>
import { ref } from 'vue'
export default {
  setup(props, { emit: e }) {
    const n = ref(0)
    e('change', n)
  }
}
</script>
```

- ESLint: `r3-emit-renamed.vue:6`.
- translation: nothing.

**Defect 4 (miss): a setup context with a default value.** The rule reads `utils.skipDefaultParamValue(params[1])`, so
`ctx = {}` is the context `ctx`. In JavaScript the parameter is an `assignment_pattern`, which has no `pattern` field,
so `fn_param` gives the pattern itself and `setup_context` (`kind(p, "identifier")`) fails.

```vue
<!-- r4-ctx-default.vue -->
<script>
import { ref } from 'vue'
export default {
  setup(props, ctx = {}) {
    const n = ref(0)
    ctx.emit('change', n)
  }
}
</script>
```

- ESLint: `r4-ctx-default.vue:6`.
- translation: nothing.

**Defect 5 (extra): nested and rest patterns of `toRefs`.** The rule follows each top-level property of the pattern;
a nested pattern (`{ a: { b } }`) is recorded as a pattern, not its names, and a rest element is no property. `ref_var`'s
`toRefs` clause takes every bound name anywhere in the pattern (`has(l, p, ref_pattern_kind)`).

```vue
<!-- r5-torefs-nested.vue -->
<script setup>
import { toRefs } from 'vue'
const props = defineProps(['a', 'c'])
const { a: { b }, ...rest } = toRefs(props)
const x = b + 1
const y = rest.c.foo
</script>
```

- ESLint: nothing.
- translation: `r5-torefs-nested.vue:5`, `:6`.

**Defect 6 (extra): a ref call under `as`.** `processDefineRef` reads the call's own parent; under `as` it is a
`TSAsExpression`, not the declarator, so the variable is not tracked. `ref_var` reads the declarator's value through
`vunwrap`.

```vue
<!-- r8-as-ref.vue -->
<script setup lang="ts">
import { ref, type Ref } from 'vue'
const n = ref(0) as Ref<number>
const m = n + 1
let k = ref(0)
k = 4
</script>
```

- ESLint: `r8-as-ref.vue:6`.
- translation: `:4`, `:6`.

**Defect 7 (miss, edge): `defineModel()` reassigned a ref.** The plugin processes the `ref()` calls first and the
`defineModel()` calls after, and each pass overwrites the references' define chain: the last pass is
`defineModel()`'s, whose call is the declarator's initialiser, so every use is reported, the reassignment too.
`remade` drops the variable on any later maker assignment, whatever the order.

```vue
<!-- r6-model-reassigned.vue -->
<script setup>
import { ref } from 'vue'
let m = defineModel()
m = ref(1)
const z = m + 1
</script>
```

- ESLint: `r6-model-reassigned.vue:4`, `:5` (`defineModel()`).
- translation: nothing.

## The author's result file

Read after the findings above. `vue-01.result.json` lists all five rules `same` and has no `different` entry, so no
`right` side needs checking against the tool. `ptcheck --ref tbelanger/pack-translations-vue-01 -- --pack vue --rules
<the five>` on pt-w07 (`deff5c564c`) agrees: 5 same, 0 different, 0 failed, on fixtures (773 files) and samples-vue
(141 files). The defects above are in cases the corpora do not have.

One stale record: `equivalence.json` still lists all five rules under `different` with "recorded by the suite, not yet
explained", right `unknown`, and originalOnly counts from before this batch (fixtures 92, 69, 15, 64, 43; samples-vue 1
for no-dupe-keys). The suite reports each as "reports the same findings as its original now; remove it from
different". The pack declares `equivalent: false`, so it does not gate, but the five entries should go.
