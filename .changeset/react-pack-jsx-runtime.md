---
"@neralabs/rerail-pack-react": patch
---

React pack: every rule skips files whose JSX belongs to another library (an `@jsx`/`@jsxImportSource` pragma naming it, `h` or `createElement` imported from Stencil, Preact, Vue, Solid, Hyperapp or Mithril, or a package that depends on Vue, Preact or another such library and not on React), so Stencil components no longer get `jsx-key` and `react-in-jsx-scope` findings and Vue's `useId` is no longer a React Hook.
