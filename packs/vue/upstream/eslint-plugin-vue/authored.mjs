// Cases Rerail wrote for the pack: inputs a port once misread. Each is `{ rule, name, code }`, a `.vue` source written
// by `generate.mjs fixtures` as fixtures/<rule>/rerail-<name>.vue and scored against what eslint-plugin-vue reports.
export const AUTHORED = [
  {
    // `toRefs(x)` is a container of refs, not a ref: its properties (and a destructuring of it) are the refs.
    rule: "no-ref-as-operand",
    name: "torefs-container",
    code: `<script setup>
import { toRefs, ref } from 'vue'
const props = defineProps({ a: Number })
const refs = toRefs(props)
const { a } = toRefs(props)
console.log(refs.a + 1)
console.log(refs.a.value)
console.log(a + 1)
const r = ref(0)
console.log(r + 1)
</script>
`,
  },
  {
    // A start tag is <script setup> by an attribute named `setup`, not by a value that says it.
    rule: "no-export-in-script-setup",
    name: "setup-in-attribute-value",
    code: `<script data-mode="setup">
export const a = 1
</script>
`,
  },
  {
    rule: "no-export-in-script-setup",
    name: "setup-after-quoted-value",
    code: `<script data-mode="x>setup" lang="ts" setup>
export const a = 1
</script>
`,
  },
  {
    rule: "no-export-in-script-setup",
    name: "setup-attribute-with-value",
    code: `<script
  setup="yes"
>
export const a = 1
</script>
`,
  },
  {
    // `defineComponent`/`component`/`createApp` define a component only when global or imported from vue.
    rule: "no-mutating-props",
    name: "local-define-component",
    code: `<script>
function defineComponent(options) { return options }
defineComponent({
  props: ['value'],
  methods: { m() { this.value = 1 } }
})
</script>
`,
  },
  {
    rule: "no-mutating-props",
    name: "other-package-define-component",
    code: `<script>
import { defineComponent } from 'elsewhere'
defineComponent({
  props: ['value'],
  methods: { m() { this.value = 1 } }
})
</script>
`,
  },
  {
    rule: "no-mutating-props",
    name: "vue-define-component",
    code: `<script>
import { defineComponent } from 'vue'
defineComponent({
  props: ['value'],
  methods: { m() { this.value = 1 } }
})
</script>
`,
  },
  {
    rule: "no-mutating-props",
    name: "global-define-component",
    code: `<script>
defineComponent({
  props: ['value'],
  methods: { m() { this.value = 1 } }
})
</script>
`,
  },
  {
    rule: "no-mutating-props",
    name: "required-define-component",
    code: `<script>
const { defineComponent } = require('elsewhere')
defineComponent({
  props: ['value'],
  methods: { m() { this.value = 1 } }
})
</script>
`,
  },
  {
    // `setup: props => {..}`: a single unparenthesized parameter is the props.
    rule: "no-mutating-props",
    name: "setup-arrow-bare-parameter",
    code: `<script>
export default {
  props: { count: Number },
  setup: props => {
    props.count++
  }
}
</script>
`,
  },
  {
    // A character reference decodes to text; one that String.prototype.trim strips is not child content.
    rule: "no-child-content",
    name: "whitespace-character-references",
    code: `<template>
  <div v-html="a">&#32;</div>
  <div v-html="a">&nbsp;</div>
  <div v-html="a">&#x20;&#x09;</div>
  <div v-html="a">&#8203;</div>
  <div v-html="a">&amp;</div>
  <div v-html="a">&ensp;&nbsp;x</div>
  <div v-html="a">&#32</div>
  <div v-text="a">&NewLine;</div>
  <div v-text="a">&zwj;</div>
  <div v-text="a">&NoBreak;</div>
  <div v-text="a">&#xFEFF;</div>
  <div v-text="a">&#133;</div>
</template>
`,
  },
  {
    rule: "require-valid-default-prop",
    name: "local-define-component",
    code: `<script>
function defineComponent(options) { return options }
defineComponent({
  props: { a: { type: String, default: 1 } }
})
</script>
`,
  },
  {
    rule: "require-valid-default-prop",
    name: "other-package-define-component",
    code: `<script>
import { defineComponent } from 'elsewhere'
defineComponent({
  props: { a: { type: String, default: 1 } }
})
</script>
`,
  },
  {
    rule: "require-valid-default-prop",
    name: "vue-define-component",
    code: `<script>
import { defineComponent } from 'vue'
defineComponent({
  props: { a: { type: String, default: 1 } }
})
</script>
`,
  },
  {
    rule: "require-valid-default-prop",
    name: "global-define-component",
    code: `<script>
defineComponent({
  props: { a: { type: String, default: 1 } }
})
</script>
`,
  },
  {
    // A self-closing element ascends to its parent for the SVG/MathML namespace; it is no custom component there.
    rule: "require-v-for-key",
    name: "self-closing-in-foreign-namespace",
    code: `<template>
  <div>
    <svg>
      <circle v-for="i in 3" />
      <circle v-for="i in 3" :key="i" />
      <g v-for="i in 3"><circle /></g>
    </svg>
    <math>
      <mi v-for="i in 3" />
      <mrow v-for="i in 3"></mrow>
    </math>
    <circle v-for="i in 3" />
    <foo-bar v-for="i in 3" />
    <math>
      <annotation-xml>
        <div v-for="i in 3" />
      </annotation-xml>
      <mi>
        <div v-for="i in 3" />
      </mi>
    </math>
  </div>
</template>
`,
  },
  {
    // The view reads a component tag as an identifier; the plugin counts only a descendant element's tag name
    // (never the element's own, nor a dotted \`Foo.Bar\`) and does not make the tag a reference.
    rule: "no-unused-vars",
    name: "component-tag-names",
    code: `<template>
  <Foo v-for="Foo in list" />
  <div v-for="Foo in list"><Foo /></div>
  <div v-for="Foo in list"><span><Foo /></span></div>
  <Foo v-for="Foo in list"><Foo /></Foo>
  <Foo v-for="Foo in list"><Foo.Bar /></Foo>
  <Foo v-for="Foo in list"></Foo>
  <Foo v-for="Foo in list" :a="Foo"></Foo>
  <Bar #default="{ Foo }"><Foo /></Bar>
  <Bar #default="{ Foo }"></Bar>
  <Bar v-slot="Foo"><Foo /></Bar>
  <Bar v-for="(Foo, Baz) in list"><Baz /></Bar>
  <Bar v-for="(a, Baz) in list"><Baz /></Bar>
  <Bar v-for="(a, b) in list"><Baz :x="b" /></Bar>
  <Bar v-for="{ a = 1 } in list"></Bar>
  <div v-for="a in list" @click="a = 2"></div>
  <div v-for="a in list">{{ a }}</div>
  <div v-for="foo in list"><foo /></div>
</template>
`,
  },
  {
    // A computed `['setup']` key names the setup function as `setup` does.
    rule: "no-setup-props-reactivity-loss",
    name: "computed-setup-method-key",
    code: `<script>
export default {
  ['setup'](props) {
    const { a } = props
  },
}
</script>
`,
  },
  {
    rule: "no-setup-props-reactivity-loss",
    name: "computed-setup-pair-key",
    code: `<script>
export default {
  ['setup']: (props) => {
    const { a } = props
  },
}
</script>
`,
  },
  {
    rule: "no-setup-props-reactivity-loss",
    name: "computed-setup-template-key",
    code: `<script>
export default {
  [\`setup\`](props) {
    const { a } = props
  },
}
</script>
`,
  },
  {
    rule: "no-setup-props-reactivity-loss",
    name: "computed-setup-dynamic-key",
    code: `<script>
export default {
  ["set" + "up"](props) {
    const { a } = props
  },
}
</script>
`,
  },
  {
    // `(defineProps(..))` is the macro call, parentheses aside.
    rule: "no-setup-props-reactivity-loss",
    name: "parenthesized-define-props",
    code: `<script setup>
const props = (defineProps({ a: String }))
const { a } = props
const q = ((defineProps({ b: String })))
const { b } = q
const w = (withDefaults(defineProps({ c: String }), {}))
const { c } = w
</script>
`,
  },
  {
    rule: "no-setup-props-reactivity-loss",
    name: "parenthesized-define-props-in-with-defaults",
    code: `<script setup>
const props = withDefaults((defineProps({ c: String })), {})
const { c } = props
const d = defineProps /* x */ ({ d: String })
const { d: dd } = d
</script>
`,
  },
  {
    // A comment is no argument: the options object is the first argument of `defineProps` though it is the second child.
    rule: "require-valid-default-prop",
    name: "comment-before-define-props-argument",
    code: `<script setup>
defineProps(/* the props */ { a: { type: String, default: 1 } })
</script>
`,
  },
  {
    // The same for `withDefaults`' two arguments.
    rule: "require-valid-default-prop",
    name: "comment-in-with-defaults",
    code: `<script setup lang="ts">
withDefaults(defineProps(/* c */ { a: { type: String, default: 1 } }), /* d */ {})
</script>
`,
  },
  {
    // The same for `defineModel`'s name and options.
    rule: "require-valid-default-prop",
    name: "comment-before-define-model-options",
    code: `<script setup>
defineModel(/* c */ 'm', /* d */ { type: String, default: 1 })
</script>
`,
  },
  {
    // The same for the last argument of a component call.
    rule: "require-valid-default-prop",
    name: "comment-around-define-component-argument",
    code: `<script>
import { defineComponent } from 'vue'
defineComponent(/* a */ {
  props: { a: { type: String, default: 1 } }
} /* b */)
</script>
`,
  },
  {
    // The same for the first argument of `new Vue`.
    rule: "require-valid-default-prop",
    name: "comment-before-new-vue-argument",
    code: `<script>
new Vue(/* c */ {
  props: { a: { type: String, default: 1 } }
})
</script>
`,
  },
  {
    // `export default {..} as const` is no component object to the plugin (isVueComponentFile wants the object itself).
    rule: "require-valid-default-prop",
    name: "export-default-as-const",
    code: `<script lang="ts">
export default {
  props: { a: { type: String, default: 1 } }
} as const
</script>
`,
  },
  {
    // `as` around the object argument of a component call is skipped.
    rule: "require-valid-default-prop",
    name: "define-component-as-argument",
    code: `<script lang="ts">
import { defineComponent } from 'vue'
export default defineComponent({
  props: { a: { type: String, default: 1 } }
} as any)
</script>
`,
  },
  {
    rule: "require-valid-default-prop",
    name: "export-default-parenthesized",
    code: `<script>
export default ({
  props: { a: { type: String, default: 1 } }
})
</script>
`,
  },
  {
    // A native tag has no read in the view, so the descendant `<div>` is not seen (declared divergence).
    rule: "no-unused-vars",
    name: "native-tag-descendant",
    code: `<template>
  <div v-for="div in list"><div /></div>
</template>
`,
  },
];
