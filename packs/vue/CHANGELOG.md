# @neralabs/rerail-pack-vue

## 0.1.5

### Patch Changes

- 7392aea: README and licence notes: the pack's test data now lives outside the package; no rule changes.
- 115b81b: Vue pack: eslint-plugin-vue is gated by port readiness, and 19 rules that pass every rule gate and the pack's conformance check ship: `attribute-hyphenation`, `no-async-in-computed-properties`, `no-child-content`, `no-deprecated-v-on-native-modifier`, `no-duplicate-attributes`, `no-export-in-script-setup`, `no-mutating-props`, `no-ref-as-operand`, `no-side-effects-in-computed-properties`, `no-template-shadow`, `no-unused-vars`, `no-use-v-if-with-v-for`, `no-v-html`, `no-v-text-v-html-on-component`, `no-watch-after-await`, `require-v-for-key`, `v-on-event-hyphenation` and `valid-v-for` on, as `flat/recommended` turns them on, and `no-setup-props-reactivity-loss` off (no recommended eslint-plugin-vue configuration turns it on). The ports report the ranges eslint-plugin-vue reports (the directive, attribute, start tag, identifier or child content, no longer a point at its start), `no-unused-vars` reads a Vue 2 filter's name as no reference, as vue-eslint-parser does, and `no-mutating-props` checks `v-model` before the prop lookup so it stays within the engine's work budget. 5 remain candidates.

## 0.1.4

### Patch Changes

- e06db22: Vue pack: messages whose rewrite only changed the voice are back to their original wording; in an eval with a coding agent, the original and rewritten messages steered it equally well.
- e06db22: Vue pack: every rule description now says in one plain sentence what the rule wants; the exact matching detail moved to the pack README.
- e06db22: Vue pack: every violation message now says in one sentence what to change in the flagged code, without tool names or rule codes.

## 0.1.3

### Patch Changes

- 1fbb06c: Vue pack candidates: script and template-expression ports read the component's program with `ast.embedded: "script-block"`, script markers and template elements through `host`, and template rules reach the program with `embedded`. `no-unused-vars` counts a native descendant tag as a use, as eslint-plugin-vue does, so its declared divergence is gone.

## 0.1.2

### Patch Changes

- da9e12b: First release through the release workflow with npm trusted publishing. The rules are unchanged.

## 0.1.1

### Patch Changes

- 2a303bf: Vue pack: ships no rules yet. Its 24 eslint-plugin-vue ports are candidates until they pass the V1 rule validation gates, and the pack is never recommended while it has no rules.
- 2a303bf: Add the Vue pack: eslint-plugin-vue ports.
