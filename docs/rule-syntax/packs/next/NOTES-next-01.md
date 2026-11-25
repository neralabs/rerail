# next-01: no-async-client-component, no-duplicate-head, no-location-assign-relative-destination

All three are `same` as their JSON originals on examples, fixtures and samples-next
(`ptcheck … --rules next-no-async-client-component,next-no-duplicate-head,next-no-location-assign-relative-destination`).

## The one cause: `earlier` is empty on JavaScript and TypeScript

Each rule ordered nodes with core's `earlier(m, n)`. core.rerail documents it as start-offset order outside Python,
but the engine lowers it to `visit_index`, and fills `visit_index` for Python documents only
(`native/src/code_rule_dsl/facts/vtab/syntax/tree.rs`, `if self.visit_index && python …`). On a .js or .tsx file
every `earlier` is false and every `!earlier`/`count : { …, earlier … } = 0` true. A probe on the async fixtures:
`visit_index(e, i)` for every `export_statement` gave no rows; `precedes(u, e)` between the same statements did.

What the rules lost, each a recorded fixture of the plugin (so the plugin reports it, the original agreed):

- no-async-client-component, all 5 invalid fixtures: `"use client"` has to come before the default export
  (`earlier(u, e)`), so nothing was reported.
  `"use client"; export default async function MyComponent() {}` → the plugin reports the export statement.
- no-duplicate-head, 3 findings: a second `<Head>` needs an earlier sibling `<Head>`.
  `<Html><Head /><Head /><Head /></Html>` → plugin reports the 2nd and 3rd.
- no-location-assign-relative-destination, invalid-026: the last write before the read decides.
  `let url = 'https://example.com'; url = '/other-path'; location.href = url` → plugin reports; the
  translation saw no write before the read and fell back to the initialiser, which is absolute.

Fix in the rules: `src_before(m, n)` from a new library, `lib/tsx/next-01.rerail` (all three rules use it): the
documented order, read off `node`'s start offsets. Every pair compared is statements, siblings, or an assignment
target and a later read, which never nest, so start offset and visit order agree.

## Library and engine changes proposed

- Engine: fill `visit_index` (start-offset preorder) for non-Python documents, as core.rerail says. Then
  `lib/tsx/next-01.rerail` goes and these rules use `earlier` again.
- Until then, every other TS library use of `earlier` is silently off: `tsx/next.rerail`'s `later_import` /
  `later_default` (so `first_import`/`default_import` take every import of a module, not the last),
  `tsx/eslint-a-control.rerail:50`, `tsx/react-compiler.rerail:210`, `tsx/ts-b-vitest.rerail:185`. Their owners'
  rules may be `same` only because no corpus file has two candidates.
