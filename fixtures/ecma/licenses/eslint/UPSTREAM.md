# ESLint 10.11.0

Upstream: [ESLint 10.11.0](https://github.com/eslint/eslint/tree/v10.11.0). License: MIT (Copyright OpenJS Foundation and other contributors, <www.openjsf.org>); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at that tag.

The files listed below contain material copied from ESLint and remain under its license. Every other file in this
pack is Rerail's, under Rerail's MIT license (`rerail/LICENSE`): the `generate_*` scripts, the `*.expected.json`
outputs recorded by running ESLint, and the fixtures and cases written for Rerail.

Paths are relative to `rerail-fixtures/ecma/`.

| File | Upstream path | Copy |
|---|---|---|
| `scope/eslint-private/cases.json` | `tests/lib/rules/no-unused-private-class-members.js` | derived: the 61 `valid/N` and `invalid/N` cases are the rule's own test cases, reformatted as JSON; the 22 `extra/N` cases are Rerail's. |
| `value/eslint/no-dupe-else-if.ts` | `tests/lib/rules/no-dupe-else-if.js` | derived: 8 of its 15 cases are the rule's own test cases; the rest are Rerail's. |
