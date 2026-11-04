---
"@neralabs/rerail-pack-javascript": patch
---

`eslint-no-unsafe-optional-chaining` no longer reports a JSX spread attribute such as `<X {...a?.b} />`, which ESLint does not report and which does not throw. Spreading an optional chain into an array or call arguments is still reported.
