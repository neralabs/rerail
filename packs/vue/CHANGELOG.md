# @neralabs/rerail-pack-vue

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
