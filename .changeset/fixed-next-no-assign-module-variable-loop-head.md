---
"@neralabs/rerail-pack-next": patch
---

`next-no-assign-module-variable` now reports `module` declared in the head of a `for…of` or `for…in` loop, such as `for (const module of mods)`, as `@next/eslint-plugin-next` does. It reports at the `const`, `let` or `var` keyword.
