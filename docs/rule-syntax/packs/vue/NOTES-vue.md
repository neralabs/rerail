# vue: 23 rules, hand-written (eslint-plugin-vue 10.11.1, vue-eslint-parser 10.4.1)

The 23 rules have 1,162 lines. They share six new library files, 695 lines in all:

| file | what |
|---|---|
| `lib/vue/elements.rr` | generated tables: HTML, SVG and MathML element names, and the SVG attributes with mixed case (`gen-vue-elements.py` writes it; `check-vue-elements.py` checks it against the plugin's JSON) |
| `lib/vue/template.rr` | the template as vue-eslint-parser reads it: elements, attributes, directives with their arguments and modifiers, namespaces, `isCustomComponent` |
| `lib/vue/code.rr` | the template's code (see "Harness" below): expressions, v-for aliases and lists, slot parameters, template variables and the names they bind, free names |
| `lib/vue/component.rr` | the script: `<script setup>`, component objects (`getVueObjectType`), options and the names they declare (`iterateProperties`), computed getters, `setup()`, Vue API calls, script-setup macros and defineProps names |
| `lib/vue/flow.rr` | whether a function body can complete normally (for return-in-computed-property) |
| `lib/vue/mutation.rr` | `utils.findMutating`: from a node, up through member accesses, to an assignment, update, `delete`, array-mutator call or `Object.assign` |

## Scores

Scored with `tools/rrlab/rrlab score vue <id> --all`, so over every rule's fixtures (773 files), not only the
rule's own. Column scores are at (path, line, column).

| rule | lines | TP/FP/FN | columns | deviations |
|---|---|---|---|---|
| attribute-hyphenation | 31 | 23/0/0 | 23/0/0 | no fix (a) |
| no-async-in-computed-properties | 82 | 96/0/0 | 96/0/0 | |
| no-child-content | 26 | 12/0/0 | 12/0/0 | (b), fixed |
| no-deprecated-v-on-native-modifier | 17 | 2/0/0 | 2/0/0 | |
| no-dupe-keys | 54 | 69/0/0 | 69/0/0 | |
| no-duplicate-attributes | 26 | 2/0/0 | 2/0/0 | |
| no-export-in-script-setup | 31 | 15/0/0 | 15/0/0 | |
| no-mutating-props | 141 | 64/0/0 | 64/0/0 | shallowOnly (e) |
| no-ref-as-operand | 94 | 44/0/0 | 44/0/0 | |
| no-setup-props-reactivity-loss | 116 | 61/0/0 | 61/0/0 | |
| no-side-effects-in-computed-properties | 49 | 51/0/0 | 51/0/0 | |
| no-template-shadow | 31 | 28/0/0 | 28/0/0 | |
| no-unused-refs | 62 | 7/0/0 | 7/0/0 | |
| no-unused-vars | 29 | 181/0/0 | 181/0/0 | |
| no-use-v-if-with-v-for | 25 | 10/0/0 | 10/0/0 | |
| no-v-html | 18 | 24/0/0 | 24/0/0 | |
| no-v-text-v-html-on-component | 23 | 17/0/0 | 17/0/0 | |
| no-watch-after-await | 29 | 5/0/0 | 5/0/0 | |
| require-v-for-key | 24 | 133/0/3 | 133/0/3 | recording duplicates (c) |
| require-valid-default-prop | 123 | 70/0/0 | 70/0/0 | TypeScript-only props (g) |
| return-in-computed-property | 34 | 56/0/0 | 56/0/0 | |
| v-on-event-hyphenation | 24 | 15/0/0 | 15/0/0 | |
| valid-v-for | 73 | 46/0/0 | 46/0/0 | unexpectedExpression (d) |

22 of 23 are perfect at line level, 22 at column level. Messages were compared with ESLint on samples for
attribute-hyphenation, v-on-event-hyphenation, no-v-html, no-v-text-v-html-on-component, no-child-content,
no-mutating-props, no-setup-props-reactivity-loss and require-valid-default-prop. Options were checked as well
where those rules take them. ESLint 10.11.0 with the plugin is installed in the upstream checkout
(`wolfeel/.rerail/port-readiness/upstream/eslint-plugin-vue/…/node_modules`). ESLint 9.39 under Data-layer-main-up
has no eslint-plugin-vue.

### Deviations

- **(a) attribute-hyphenation, no fix.** The plugin's fix renames the key, but the policy JSON has no fix, so none
  is offered.
- **(b) no-child-content, column (fixed).** ESLint reports the children's range, which starts right after the start
  tag's `>`. The finding first sat on the `>` token, one column early. It now sits on a regex capture over the
  element's text, the first character after the start tag (quoted attribute values skipped), which rrlab models as
  a node since its update: 12/0/0 at columns too.
- **(c) require-v-for-key, 3 FN.** These come from the recording, not the rule. ESLint reports a key-less element
  inside a nested `<template v-for>` twice (once per enclosing v-for), and the recording keeps both rows. The rule
  reports each element once. The three are no-mutating-props/invalid-006.vue:6 and valid-001.vue:17 and :18.
- **(d) valid-v-for, unexpectedExpression dropped.** vue-eslint-parser reports `v-for="x in"` and the like as a
  parse error, so the rule's own message never shows. The JSON has no such clause either.
- **(e) no-mutating-props.** Only the default options are covered, as in the JSON: `shallowOnly` is not offered.
- **(g) require-valid-default-prop.** Props declared only by a TypeScript type (`defineProps<T>()`,
  `defineModel<T>()`) are not read, as the JSON says: their runtime types need the plugin's `inferRuntimeType`. The
  recording has none of these: those upstream tests were left out of the fixtures.

## Harness (rrlab)

These changes are in `tools/rrlab`, and the README's Vue paragraph now describes them:

- **`src/main.rs`.** A Vue file's code is parsed again as guest trees of the same document, at the file's own lines
  and columns. Each guest has a core `parsed(host, grammar, root)` row. The roles are:
  - "javascript", "typescript" or "tsx": a `<script>`, by its `lang`;
  - "vue-expression": a directive value, a `{{ }}`'s text or a dynamic argument;
  - "vue-statements": a v-on value;
  - "vue-params": v-slot, `#x`, slot-scope or scope;
  - "vue-for-alias" and "vue-for-source": a v-for's alias and its list.
  `lib/vue/code.rr` documents the wrappers.
- **`lower.py`.** "parsed" is now filled. A rule whose `.language` is not vue still gets the "not modelled"
  warning for it. A `.violation` message's `\"` and `\\` are unescaped. Before this, rrlab printed `\"` in
  messages; CV05 in the SQL pack had the same artefact.

**Engine proposal.** The engine should give `parsed` rows for a Vue file's script and template code, with the
roles above. Today it has one view program, reached through `ast.embedded` and `host`. The vue libraries depend
only on `parsed`, `kind`, `field`, `child` and core's axes. Guest positions are the host's.

## Review fixes

The review (ESLint 10.11 on the reviewer's cases in scratchpad/rev-vue/cases) found eight defects. Seven are fixed,
and each fix was checked against ESLint on those cases:

- **Component definers.** `component.rr` now has `vdefiner_callee`, as isGlobalOrImportedFromVue: a callee counts
  unless it is a named import from another package, a function, class or catch binding, or
  `const { f } = require("other")`. `vfrom_vue` stays strict, for `vvue_call`.
- **Macros.** `vmacro` counts a macro only as a top-level declarator's value or a top-level expression statement
  of `<script setup>`, and defineProps also inside withDefaults there. `let p; p = defineProps()` is no macro, so
  reactivity-loss no longer reads assignments.
- **valid-v-for.** An alias's slot counts the commas before it, holes included. Only slots 1 and 2 must be plain
  names, and a hole no longer turns that check off.
- **reactivity-loss.** A `for` is a scope only when it declares with `let` or `const`.
- **no-mutating-props.** Without an export or setup props, the template uses the component object exited last.
- **no-ref-as-operand.** A variable that is later assigned a new ref (`b = ref(2)`) is skipped, as the plugin
  does.
- **mutation.rr.** `Object.assign` needs a plain `.assign`, so `Object['assign']` is not counted. The `!` clause
  of `vmut_strip` is gone: `estree_parent` stops at a non-null expression anyway.

The eighth, no-child-content's column (b), is fixed since rrlab models regex captures as spans.

Idioms: `vref_site` and `local()` are gone (names.rr resolves once now). `vvue_import_call` is defined directly,
and `earlier_hit` uses `earlier`.

Open, not confirmed: vue-eslint-parser tries a v-on value as an expression first, so `@click="function(){}"` is an
expression there. rrlab always parses a v-on value as statements.

## Library and core changes

- None outstanding.
- No duplicates to merge. All new names are prefixed `v`/`t` in `lib/vue/`. The rules' private helpers do not
  clash with core: check_rr reports nothing for packs/vue or lib/vue.
