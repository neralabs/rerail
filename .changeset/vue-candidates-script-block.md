---
"@neralabs/rerail-pack-vue": patch
---

Vue pack candidates: script and template-expression ports read the component's program with `ast.embedded: "script-block"`, script markers and template elements through `host`, and template rules reach the program with `embedded`. `no-unused-vars` counts a native descendant tag as a use, as eslint-plugin-vue does, so its declared divergence is gone.
