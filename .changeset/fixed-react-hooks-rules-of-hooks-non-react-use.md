---
"@neralabs/rerail-pack-react": patch
---

`react-hooks-rules-of-hooks` no longer reports a `use` call the file binds to something other than React, such as chai's imported `use(plugin)`, markdown-it's `Md.use(plugin)` or a test fixture's `use` parameter, as a Hook called outside a component. React's `use`, imported from `react` or unbound, is still checked.
