# eslint-plugin-react 7.37.5

Upstream: [jsx-eslint/eslint-plugin-react](https://github.com/jsx-eslint/eslint-plugin-react/tree/2c98b83c451a4297edf1787d9a616e50687e27e8) at tag `v7.37.5` (commit `2c98b83c451a4297edf1787d9a616e50687e27e8`). License: MIT (Copyright (c) 2014 Yannick Croissant); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at that commit.

The files listed below are the upstream tests of the rules the React pack ports (`conformance.json`, `upstream/common.mjs` `RULES`), and the test cases extracted from them, one case per file, and remain under eslint-plugin-react's license. Every other file under `upstream/eslint-plugin-react/` is Rerail's, under the Apache License 2.0 (the pack's `LICENSE`): `generate.mjs` (with its stub RuleTester), `../common.mjs` and the expected findings they write by running the pinned plugin. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of eslint-plugin-react's and eslint-plugin-react-hooks' rules, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them.

Paths are relative to the pack directory, `packs/react/`. `<file> valid[i]` / `invalid[i]` is the i-th case of the `valid` / `invalid` list the test file passes to `RuleTester.run`. A `derived` row is a rename variant of the extracted case it names.

| File | Upstream path | Copy |
|---|---|---|
| `upstream/eslint-plugin-react/tests/lib/rules/button-has-type.js` | `tests/lib/rules/button-has-type.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/jsx-key.js` | `tests/lib/rules/jsx-key.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/jsx-no-comment-textnodes.js` | `tests/lib/rules/jsx-no-comment-textnodes.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/jsx-no-duplicate-props.js` | `tests/lib/rules/jsx-no-duplicate-props.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/jsx-no-target-blank.js` | `tests/lib/rules/jsx-no-target-blank.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/jsx-no-undef.js` | `tests/lib/rules/jsx-no-undef.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/no-array-index-key.js` | `tests/lib/rules/no-array-index-key.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/no-children-prop.js` | `tests/lib/rules/no-children-prop.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/no-danger.js` | `tests/lib/rules/no-danger.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/no-danger-with-children.js` | `tests/lib/rules/no-danger-with-children.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/no-direct-mutation-state.js` | `tests/lib/rules/no-direct-mutation-state.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/no-find-dom-node.js` | `tests/lib/rules/no-find-dom-node.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/no-is-mounted.js` | `tests/lib/rules/no-is-mounted.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/no-render-return-value.js` | `tests/lib/rules/no-render-return-value.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/no-string-refs.js` | `tests/lib/rules/no-string-refs.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/no-unescaped-entities.js` | `tests/lib/rules/no-unescaped-entities.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/react-in-jsx-scope.js` | `tests/lib/rules/react-in-jsx-scope.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/self-closing-comp.js` | `tests/lib/rules/self-closing-comp.js` | verbatim |
| `upstream/eslint-plugin-react/tests/lib/rules/void-dom-elements-no-children.js` | `tests/lib/rules/void-dom-elements-no-children.js` | verbatim |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-000.jsx` | `tests/lib/rules/button-has-type.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-001.jsx` | `tests/lib/rules/button-has-type.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-002.jsx` | `tests/lib/rules/button-has-type.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-003.jsx` | `tests/lib/rules/button-has-type.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-004.jsx` | `tests/lib/rules/button-has-type.js valid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-005.jsx` | `tests/lib/rules/button-has-type.js valid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-006.jsx` | `tests/lib/rules/button-has-type.js valid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-007.jsx` | `tests/lib/rules/button-has-type.js valid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-008.jsx` | `tests/lib/rules/button-has-type.js valid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-009.jsx` | `tests/lib/rules/button-has-type.js valid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-010.jsx` | `tests/lib/rules/button-has-type.js valid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-011.jsx` | `tests/lib/rules/button-has-type.js valid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-012.jsx` | `tests/lib/rules/button-has-type.js valid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-013.jsx` | `tests/lib/rules/button-has-type.js valid[13]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-014.jsx` | `tests/lib/rules/button-has-type.js valid[14]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-015.jsx` | `tests/lib/rules/button-has-type.js valid[15]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-016.jsx` | `tests/lib/rules/button-has-type.js valid[16]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-017.jsx` | `tests/lib/rules/button-has-type.js valid[17]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-018.jsx` | `tests/lib/rules/button-has-type.js valid[18]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-019.jsx` | `tests/lib/rules/button-has-type.js valid[19]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-020.jsx` | `tests/lib/rules/button-has-type.js valid[20]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-021.jsx` | `tests/lib/rules/button-has-type.js valid[21]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-022.jsx` | `tests/lib/rules/button-has-type.js valid[22]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-023.jsx` | `tests/lib/rules/button-has-type.js valid[23]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-024.jsx` | `tests/lib/rules/button-has-type.js valid[24]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-025.jsx` | `tests/lib/rules/button-has-type.js valid[25]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-026.jsx` | `tests/lib/rules/button-has-type.js valid[26]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-027.jsx` | `tests/lib/rules/button-has-type.js valid[27]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-028.jsx` | `tests/lib/rules/button-has-type.js valid[28]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/valid-029.tsx` | `tests/lib/rules/button-has-type.js valid[29]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-000.jsx` | `tests/lib/rules/button-has-type.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-001.jsx` | `tests/lib/rules/button-has-type.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-002.jsx` | `tests/lib/rules/button-has-type.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-003.jsx` | `tests/lib/rules/button-has-type.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-004.jsx` | `tests/lib/rules/button-has-type.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-005.jsx` | `tests/lib/rules/button-has-type.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-006.jsx` | `tests/lib/rules/button-has-type.js invalid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-007.jsx` | `tests/lib/rules/button-has-type.js invalid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-008.jsx` | `tests/lib/rules/button-has-type.js invalid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-009.jsx` | `tests/lib/rules/button-has-type.js invalid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-010.jsx` | `tests/lib/rules/button-has-type.js invalid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-011.jsx` | `tests/lib/rules/button-has-type.js invalid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-012.jsx` | `tests/lib/rules/button-has-type.js invalid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-013.jsx` | `tests/lib/rules/button-has-type.js invalid[13]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-014.jsx` | `tests/lib/rules/button-has-type.js invalid[14]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-015.jsx` | `tests/lib/rules/button-has-type.js invalid[15]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-016.jsx` | `tests/lib/rules/button-has-type.js invalid[16]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-017.jsx` | `tests/lib/rules/button-has-type.js invalid[17]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-018.jsx` | `tests/lib/rules/button-has-type.js invalid[18]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-019.jsx` | `tests/lib/rules/button-has-type.js invalid[19]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-020.jsx` | `tests/lib/rules/button-has-type.js invalid[20]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-021.jsx` | `tests/lib/rules/button-has-type.js invalid[21]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-022.jsx` | `tests/lib/rules/button-has-type.js invalid[22]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-023.jsx` | `tests/lib/rules/button-has-type.js invalid[23]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-024.jsx` | `tests/lib/rules/button-has-type.js invalid[24]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-025.jsx` | `tests/lib/rules/button-has-type.js invalid[25]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-026.jsx` | `tests/lib/rules/button-has-type.js invalid[26]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/button-has-type/invalid-027.jsx` | `tests/lib/rules/button-has-type.js invalid[27]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-000.jsx` | `tests/lib/rules/jsx-key.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-001.jsx` | `tests/lib/rules/jsx-key.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-002.jsx` | `tests/lib/rules/jsx-key.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-003.jsx` | `tests/lib/rules/jsx-key.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-004.jsx` | `tests/lib/rules/jsx-key.js valid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-005.jsx` | `tests/lib/rules/jsx-key.js valid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-006.jsx` | `tests/lib/rules/jsx-key.js valid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-007.jsx` | `tests/lib/rules/jsx-key.js valid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-008.jsx` | `tests/lib/rules/jsx-key.js valid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-009.jsx` | `tests/lib/rules/jsx-key.js valid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-010.jsx` | `tests/lib/rules/jsx-key.js valid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-011.jsx` | `tests/lib/rules/jsx-key.js valid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-012.jsx` | `tests/lib/rules/jsx-key.js valid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-013.jsx` | `tests/lib/rules/jsx-key.js valid[13]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-014.jsx` | `tests/lib/rules/jsx-key.js valid[14]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-015.jsx` | `tests/lib/rules/jsx-key.js valid[15]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-016.jsx` | `tests/lib/rules/jsx-key.js valid[16]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-017.jsx` | `tests/lib/rules/jsx-key.js valid[17]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-018.jsx` | `tests/lib/rules/jsx-key.js valid[18]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-019.jsx` | `tests/lib/rules/jsx-key.js valid[19]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-020.jsx` | `tests/lib/rules/jsx-key.js valid[20]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-021.jsx` | `tests/lib/rules/jsx-key.js valid[21]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-022.jsx` | `tests/lib/rules/jsx-key.js valid[22]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-023.jsx` | `tests/lib/rules/jsx-key.js valid[23]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-024.jsx` | `tests/lib/rules/jsx-key.js valid[24]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-025.tsx` | `tests/lib/rules/jsx-key.js valid[25]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-026.jsx` | `tests/lib/rules/jsx-key.js valid[26]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-027.jsx` | `tests/lib/rules/jsx-key.js valid[27]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-028.jsx` | `tests/lib/rules/jsx-key.js valid[28]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-029.tsx` | `tests/lib/rules/jsx-key.js valid[29]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-030.jsx` | `tests/lib/rules/jsx-key.js valid[30]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-031.jsx` | `tests/lib/rules/jsx-key.js valid[31]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/valid-032.jsx` | `tests/lib/rules/jsx-key.js valid[32]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-000.jsx` | `tests/lib/rules/jsx-key.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-001.jsx` | `tests/lib/rules/jsx-key.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-002.jsx` | `tests/lib/rules/jsx-key.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-003.jsx` | `tests/lib/rules/jsx-key.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-004.jsx` | `tests/lib/rules/jsx-key.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-005.jsx` | `tests/lib/rules/jsx-key.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-006.jsx` | `tests/lib/rules/jsx-key.js invalid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-007.jsx` | `tests/lib/rules/jsx-key.js invalid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-008.jsx` | `tests/lib/rules/jsx-key.js invalid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-009.jsx` | `tests/lib/rules/jsx-key.js invalid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-010.jsx` | `tests/lib/rules/jsx-key.js invalid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-011.jsx` | `tests/lib/rules/jsx-key.js invalid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-012.tsx` | `tests/lib/rules/jsx-key.js invalid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-013.tsx` | `tests/lib/rules/jsx-key.js invalid[13]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-014.jsx` | `tests/lib/rules/jsx-key.js invalid[14]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-015.jsx` | `tests/lib/rules/jsx-key.js invalid[15]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-016.jsx` | `tests/lib/rules/jsx-key.js invalid[16]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-017.jsx` | `tests/lib/rules/jsx-key.js invalid[17]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-018.jsx` | `tests/lib/rules/jsx-key.js invalid[18]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-019.jsx` | `tests/lib/rules/jsx-key.js invalid[19]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-020.jsx` | `tests/lib/rules/jsx-key.js invalid[20]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-021.jsx` | `tests/lib/rules/jsx-key.js invalid[21]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-022.jsx` | `tests/lib/rules/jsx-key.js invalid[22]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-key/invalid-023.jsx` | `tests/lib/rules/jsx-key.js invalid[23]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-000.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-001.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-002.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-003.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-004.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-005.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-006.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-007.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-008.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-009.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-010.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-011.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-012.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-013.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[13]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-014.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[14]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-015.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[15]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/valid-016.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[16]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/invalid-000.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/invalid-001.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/invalid-002.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/invalid-003.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/invalid-004.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/invalid-005.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/invalid-006.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js invalid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/valid-000.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/valid-001.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/valid-002.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/valid-003.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/valid-004.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/valid-005.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/valid-006.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/valid-007.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/valid-008.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/valid-009.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/valid-010.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/valid-011.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/valid-012.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/invalid-000.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/invalid-001.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/invalid-002.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/invalid-003.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/invalid-004.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/invalid-005.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-000.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-001.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-002.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-003.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-004.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-005.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-006.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-007.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-008.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-009.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-010.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-011.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-012.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-013.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[13]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-014.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[14]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-015.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[15]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-016.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[16]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-017.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[17]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-018.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[18]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-019.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[19]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-020.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[20]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-021.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[21]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-022.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[22]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-023.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[23]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-024.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[24]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-025.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[25]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-026.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[26]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-027.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[27]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-028.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[28]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-029.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[29]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-030.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[30]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-031.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[31]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-032.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[32]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-033.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[33]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-034.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[34]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-035.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[35]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-036.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[36]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-037.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[37]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-038.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[38]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-039.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[39]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-040.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[40]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-041.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[41]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-042.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[42]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-043.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[43]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-044.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[44]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-045.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[45]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-046.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[46]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-047.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[47]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-048.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[48]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-049.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[49]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-050.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[50]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-051.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[51]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-052.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[52]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-053.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[53]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-054.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[54]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-055.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[55]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-056.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[56]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-057.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[57]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-058.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[58]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-059.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[59]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-060.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[60]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-061.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[61]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/valid-062.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[62]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-000.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-001.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-002.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-003.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-004.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-005.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-006.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-007.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-008.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-009.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-010.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-011.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-012.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-013.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[13]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-014.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[14]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-015.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[15]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-016.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[16]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-017.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[17]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-018.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[18]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-019.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[19]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-020.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[20]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-021.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[21]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-022.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[22]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-023.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[23]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-024.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[24]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-025.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[25]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-026.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[26]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-027.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[27]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-028.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[28]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-029.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[29]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-030.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[30]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-031.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[31]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-032.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[32]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-033.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[33]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-034.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[34]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-035.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[35]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-036.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[36]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-037.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[37]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-038.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[38]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-039.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[39]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-040.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[40]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-041.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[41]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-042.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[42]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-043.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[43]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-044.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[44]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-045.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[45]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-046.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[46]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-047.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[47]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-048.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[48]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/invalid-049.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[49]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/valid-000.jsx` | `tests/lib/rules/jsx-no-undef.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/valid-001.jsx` | `tests/lib/rules/jsx-no-undef.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/valid-002.jsx` | `tests/lib/rules/jsx-no-undef.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/valid-003.jsx` | `tests/lib/rules/jsx-no-undef.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/valid-004.jsx` | `tests/lib/rules/jsx-no-undef.js valid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/valid-005.jsx` | `tests/lib/rules/jsx-no-undef.js valid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/valid-006.jsx` | `tests/lib/rules/jsx-no-undef.js valid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/valid-007.jsx` | `tests/lib/rules/jsx-no-undef.js valid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/valid-008.jsx` | `tests/lib/rules/jsx-no-undef.js valid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/invalid-000.jsx` | `tests/lib/rules/jsx-no-undef.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/invalid-001.jsx` | `tests/lib/rules/jsx-no-undef.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/invalid-002.jsx` | `tests/lib/rules/jsx-no-undef.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/invalid-003.jsx` | `tests/lib/rules/jsx-no-undef.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/invalid-004.jsx` | `tests/lib/rules/jsx-no-undef.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/invalid-005.jsx` | `tests/lib/rules/jsx-no-undef.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-000.jsx` | `tests/lib/rules/no-array-index-key.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-001.jsx` | `tests/lib/rules/no-array-index-key.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-002.jsx` | `tests/lib/rules/no-array-index-key.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-003.jsx` | `tests/lib/rules/no-array-index-key.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-004.jsx` | `tests/lib/rules/no-array-index-key.js valid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-005.jsx` | `tests/lib/rules/no-array-index-key.js valid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-006.jsx` | `tests/lib/rules/no-array-index-key.js valid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-007.jsx` | `tests/lib/rules/no-array-index-key.js valid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-008.jsx` | `tests/lib/rules/no-array-index-key.js valid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-009.jsx` | `tests/lib/rules/no-array-index-key.js valid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-010.jsx` | `tests/lib/rules/no-array-index-key.js valid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-011.jsx` | `tests/lib/rules/no-array-index-key.js valid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-012.jsx` | `tests/lib/rules/no-array-index-key.js valid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-013.jsx` | `tests/lib/rules/no-array-index-key.js valid[13]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-014.jsx` | `tests/lib/rules/no-array-index-key.js valid[14]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-015.jsx` | `tests/lib/rules/no-array-index-key.js valid[15]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-016.jsx` | `tests/lib/rules/no-array-index-key.js valid[16]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-017.jsx` | `tests/lib/rules/no-array-index-key.js valid[17]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-018.jsx` | `tests/lib/rules/no-array-index-key.js valid[18]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-019.jsx` | `tests/lib/rules/no-array-index-key.js valid[19]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-020.jsx` | `tests/lib/rules/no-array-index-key.js valid[20]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-021.jsx` | `tests/lib/rules/no-array-index-key.js valid[21]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-022.jsx` | `tests/lib/rules/no-array-index-key.js valid[22]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-023.jsx` | `tests/lib/rules/no-array-index-key.js valid[23]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-024.jsx` | `tests/lib/rules/no-array-index-key.js valid[24]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-025.jsx` | `tests/lib/rules/no-array-index-key.js valid[25]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-026.jsx` | `tests/lib/rules/no-array-index-key.js valid[26]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-027.jsx` | `tests/lib/rules/no-array-index-key.js valid[27]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-028.jsx` | `tests/lib/rules/no-array-index-key.js valid[28]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/valid-029.jsx` | `tests/lib/rules/no-array-index-key.js valid[29]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-000.jsx` | `tests/lib/rules/no-array-index-key.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-001.jsx` | `tests/lib/rules/no-array-index-key.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-002.jsx` | `tests/lib/rules/no-array-index-key.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-003.jsx` | `tests/lib/rules/no-array-index-key.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-004.jsx` | `tests/lib/rules/no-array-index-key.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-005.jsx` | `tests/lib/rules/no-array-index-key.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-006.jsx` | `tests/lib/rules/no-array-index-key.js invalid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-007.jsx` | `tests/lib/rules/no-array-index-key.js invalid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-008.jsx` | `tests/lib/rules/no-array-index-key.js invalid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-009.jsx` | `tests/lib/rules/no-array-index-key.js invalid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-010.jsx` | `tests/lib/rules/no-array-index-key.js invalid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-011.jsx` | `tests/lib/rules/no-array-index-key.js invalid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-012.jsx` | `tests/lib/rules/no-array-index-key.js invalid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-013.jsx` | `tests/lib/rules/no-array-index-key.js invalid[13]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-014.jsx` | `tests/lib/rules/no-array-index-key.js invalid[14]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-015.jsx` | `tests/lib/rules/no-array-index-key.js invalid[15]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-016.jsx` | `tests/lib/rules/no-array-index-key.js invalid[16]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-017.jsx` | `tests/lib/rules/no-array-index-key.js invalid[17]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-018.jsx` | `tests/lib/rules/no-array-index-key.js invalid[18]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-019.jsx` | `tests/lib/rules/no-array-index-key.js invalid[19]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-020.jsx` | `tests/lib/rules/no-array-index-key.js invalid[20]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-021.jsx` | `tests/lib/rules/no-array-index-key.js invalid[21]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-022.jsx` | `tests/lib/rules/no-array-index-key.js invalid[22]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-023.jsx` | `tests/lib/rules/no-array-index-key.js invalid[23]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-024.jsx` | `tests/lib/rules/no-array-index-key.js invalid[24]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-025.jsx` | `tests/lib/rules/no-array-index-key.js invalid[25]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-026.jsx` | `tests/lib/rules/no-array-index-key.js invalid[26]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-027.jsx` | `tests/lib/rules/no-array-index-key.js invalid[27]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-028.jsx` | `tests/lib/rules/no-array-index-key.js invalid[28]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-029.jsx` | `tests/lib/rules/no-array-index-key.js invalid[29]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-030.jsx` | `tests/lib/rules/no-array-index-key.js invalid[30]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-031.jsx` | `tests/lib/rules/no-array-index-key.js invalid[31]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-032.jsx` | `tests/lib/rules/no-array-index-key.js invalid[32]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-033.jsx` | `tests/lib/rules/no-array-index-key.js invalid[33]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-034.jsx` | `tests/lib/rules/no-array-index-key.js invalid[34]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-035.jsx` | `tests/lib/rules/no-array-index-key.js invalid[35]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-036.jsx` | `tests/lib/rules/no-array-index-key.js invalid[36]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-037.jsx` | `tests/lib/rules/no-array-index-key.js invalid[37]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-038.jsx` | `tests/lib/rules/no-array-index-key.js invalid[38]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-039.jsx` | `tests/lib/rules/no-array-index-key.js invalid[39]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/invalid-040.jsx` | `tests/lib/rules/no-array-index-key.js invalid[40]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-000.jsx` | `tests/lib/rules/no-children-prop.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-001.jsx` | `tests/lib/rules/no-children-prop.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-002.jsx` | `tests/lib/rules/no-children-prop.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-003.jsx` | `tests/lib/rules/no-children-prop.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-004.jsx` | `tests/lib/rules/no-children-prop.js valid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-005.jsx` | `tests/lib/rules/no-children-prop.js valid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-006.jsx` | `tests/lib/rules/no-children-prop.js valid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-007.jsx` | `tests/lib/rules/no-children-prop.js valid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-008.jsx` | `tests/lib/rules/no-children-prop.js valid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-009.jsx` | `tests/lib/rules/no-children-prop.js valid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-010.jsx` | `tests/lib/rules/no-children-prop.js valid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-011.jsx` | `tests/lib/rules/no-children-prop.js valid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-012.jsx` | `tests/lib/rules/no-children-prop.js valid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-013.jsx` | `tests/lib/rules/no-children-prop.js valid[13]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-014.jsx` | `tests/lib/rules/no-children-prop.js valid[14]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-015.jsx` | `tests/lib/rules/no-children-prop.js valid[15]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-016.jsx` | `tests/lib/rules/no-children-prop.js valid[16]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-017.jsx` | `tests/lib/rules/no-children-prop.js valid[17]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-018.jsx` | `tests/lib/rules/no-children-prop.js valid[18]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-019.jsx` | `tests/lib/rules/no-children-prop.js valid[19]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-020.jsx` | `tests/lib/rules/no-children-prop.js valid[20]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-021.jsx` | `tests/lib/rules/no-children-prop.js valid[21]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-022.jsx` | `tests/lib/rules/no-children-prop.js valid[22]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-023.jsx` | `tests/lib/rules/no-children-prop.js valid[23]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-024.jsx` | `tests/lib/rules/no-children-prop.js valid[24]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-025.jsx` | `tests/lib/rules/no-children-prop.js valid[25]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-026.jsx` | `tests/lib/rules/no-children-prop.js valid[26]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-027.jsx` | `tests/lib/rules/no-children-prop.js valid[27]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-028.jsx` | `tests/lib/rules/no-children-prop.js valid[28]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-029.jsx` | `tests/lib/rules/no-children-prop.js valid[29]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-030.jsx` | `tests/lib/rules/no-children-prop.js valid[30]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-031.jsx` | `tests/lib/rules/no-children-prop.js valid[31]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-032.jsx` | `tests/lib/rules/no-children-prop.js valid[32]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-033.jsx` | `tests/lib/rules/no-children-prop.js valid[33]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-034.jsx` | `tests/lib/rules/no-children-prop.js valid[34]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-035.jsx` | `tests/lib/rules/no-children-prop.js valid[35]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-036.jsx` | `tests/lib/rules/no-children-prop.js valid[36]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-037.jsx` | `tests/lib/rules/no-children-prop.js valid[37]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-038.jsx` | `tests/lib/rules/no-children-prop.js valid[38]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-039.jsx` | `tests/lib/rules/no-children-prop.js valid[39]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-040.jsx` | `tests/lib/rules/no-children-prop.js valid[40]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-041.jsx` | `tests/lib/rules/no-children-prop.js valid[41]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-042.jsx` | `tests/lib/rules/no-children-prop.js valid[42]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-043.jsx` | `tests/lib/rules/no-children-prop.js valid[43]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/valid-044.jsx` | `tests/lib/rules/no-children-prop.js valid[44]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-000.jsx` | `tests/lib/rules/no-children-prop.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-001.jsx` | `tests/lib/rules/no-children-prop.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-002.jsx` | `tests/lib/rules/no-children-prop.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-003.jsx` | `tests/lib/rules/no-children-prop.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-004.jsx` | `tests/lib/rules/no-children-prop.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-005.jsx` | `tests/lib/rules/no-children-prop.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-006.jsx` | `tests/lib/rules/no-children-prop.js invalid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-007.jsx` | `tests/lib/rules/no-children-prop.js invalid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-008.jsx` | `tests/lib/rules/no-children-prop.js invalid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-009.jsx` | `tests/lib/rules/no-children-prop.js invalid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-010.jsx` | `tests/lib/rules/no-children-prop.js invalid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-011.jsx` | `tests/lib/rules/no-children-prop.js invalid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-012.jsx` | `tests/lib/rules/no-children-prop.js invalid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-013.jsx` | `tests/lib/rules/no-children-prop.js invalid[13]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-014.jsx` | `tests/lib/rules/no-children-prop.js invalid[14]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-015.jsx` | `tests/lib/rules/no-children-prop.js invalid[15]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-016.jsx` | `tests/lib/rules/no-children-prop.js invalid[16]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-017.jsx` | `tests/lib/rules/no-children-prop.js invalid[17]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-018.jsx` | `tests/lib/rules/no-children-prop.js invalid[18]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-019.jsx` | `tests/lib/rules/no-children-prop.js invalid[19]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-020.jsx` | `tests/lib/rules/no-children-prop.js invalid[20]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-021.jsx` | `tests/lib/rules/no-children-prop.js invalid[21]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/invalid-022.jsx` | `tests/lib/rules/no-children-prop.js invalid[22]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger/valid-000.jsx` | `tests/lib/rules/no-danger.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger/valid-001.jsx` | `tests/lib/rules/no-danger.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger/valid-002.jsx` | `tests/lib/rules/no-danger.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger/valid-003.jsx` | `tests/lib/rules/no-danger.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger/valid-004.jsx` | `tests/lib/rules/no-danger.js valid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger/valid-005.jsx` | `tests/lib/rules/no-danger.js valid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger/invalid-000.jsx` | `tests/lib/rules/no-danger.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger/invalid-001.jsx` | `tests/lib/rules/no-danger.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger/invalid-002.jsx` | `tests/lib/rules/no-danger.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger/invalid-003.jsx` | `tests/lib/rules/no-danger.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger/invalid-004.jsx` | `tests/lib/rules/no-danger.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger/invalid-005.jsx` | `tests/lib/rules/no-danger.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger/invalid-006.tsx` | `tests/lib/rules/no-danger.js invalid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-000.jsx` | `tests/lib/rules/no-danger-with-children.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-001.jsx` | `tests/lib/rules/no-danger-with-children.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-002.jsx` | `tests/lib/rules/no-danger-with-children.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-003.jsx` | `tests/lib/rules/no-danger-with-children.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-004.jsx` | `tests/lib/rules/no-danger-with-children.js valid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-005.jsx` | `tests/lib/rules/no-danger-with-children.js valid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-006.jsx` | `tests/lib/rules/no-danger-with-children.js valid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-007.jsx` | `tests/lib/rules/no-danger-with-children.js valid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-008.jsx` | `tests/lib/rules/no-danger-with-children.js valid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-009.jsx` | `tests/lib/rules/no-danger-with-children.js valid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-010.jsx` | `tests/lib/rules/no-danger-with-children.js valid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-011.jsx` | `tests/lib/rules/no-danger-with-children.js valid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-012.jsx` | `tests/lib/rules/no-danger-with-children.js valid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-013.jsx` | `tests/lib/rules/no-danger-with-children.js valid[13]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-014.jsx` | `tests/lib/rules/no-danger-with-children.js valid[14]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-015.jsx` | `tests/lib/rules/no-danger-with-children.js valid[15]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/valid-016.jsx` | `tests/lib/rules/no-danger-with-children.js valid[16]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/invalid-000.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/invalid-001.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/invalid-002.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/invalid-003.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/invalid-004.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/invalid-005.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/invalid-006.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/invalid-007.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/invalid-008.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/invalid-009.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/invalid-010.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/invalid-011.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/invalid-012.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/invalid-013.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[13]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/valid-000.jsx` | `tests/lib/rules/no-direct-mutation-state.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/valid-001.jsx` | `tests/lib/rules/no-direct-mutation-state.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/valid-002.jsx` | `tests/lib/rules/no-direct-mutation-state.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/valid-003.jsx` | `tests/lib/rules/no-direct-mutation-state.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/valid-004.jsx` | `tests/lib/rules/no-direct-mutation-state.js valid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/valid-005.jsx` | `tests/lib/rules/no-direct-mutation-state.js valid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/valid-006.jsx` | `tests/lib/rules/no-direct-mutation-state.js valid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/invalid-000.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/invalid-001.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/invalid-002.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/invalid-003.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/invalid-004.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/invalid-005.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/invalid-006.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/invalid-007.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/invalid-008.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/invalid-009.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/invalid-010.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/invalid-011.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/invalid-012.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/invalid-013.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[13]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/valid-000.jsx` | `tests/lib/rules/no-find-dom-node.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/valid-001.jsx` | `tests/lib/rules/no-find-dom-node.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/valid-002.jsx` | `tests/lib/rules/no-find-dom-node.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/valid-003.jsx` | `tests/lib/rules/no-find-dom-node.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/invalid-000.jsx` | `tests/lib/rules/no-find-dom-node.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/invalid-001.jsx` | `tests/lib/rules/no-find-dom-node.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/invalid-002.jsx` | `tests/lib/rules/no-find-dom-node.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/invalid-003.jsx` | `tests/lib/rules/no-find-dom-node.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/valid-000.jsx` | `tests/lib/rules/no-is-mounted.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/valid-001.jsx` | `tests/lib/rules/no-is-mounted.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/valid-002.jsx` | `tests/lib/rules/no-is-mounted.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/valid-003.jsx` | `tests/lib/rules/no-is-mounted.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/invalid-000.jsx` | `tests/lib/rules/no-is-mounted.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/invalid-001.jsx` | `tests/lib/rules/no-is-mounted.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/invalid-002.jsx` | `tests/lib/rules/no-is-mounted.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/valid-000.jsx` | `tests/lib/rules/no-render-return-value.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/valid-001.jsx` | `tests/lib/rules/no-render-return-value.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/valid-002.jsx` | `tests/lib/rules/no-render-return-value.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/valid-003.jsx` | `tests/lib/rules/no-render-return-value.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/valid-004.jsx` | `tests/lib/rules/no-render-return-value.js valid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/valid-005.jsx` | `tests/lib/rules/no-render-return-value.js valid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/valid-006.jsx` | `tests/lib/rules/no-render-return-value.js valid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/valid-007.jsx` | `tests/lib/rules/no-render-return-value.js valid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/invalid-000.jsx` | `tests/lib/rules/no-render-return-value.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/invalid-001.jsx` | `tests/lib/rules/no-render-return-value.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/invalid-002.jsx` | `tests/lib/rules/no-render-return-value.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/invalid-003.jsx` | `tests/lib/rules/no-render-return-value.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/invalid-004.jsx` | `tests/lib/rules/no-render-return-value.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/invalid-005.jsx` | `tests/lib/rules/no-render-return-value.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/invalid-006.jsx` | `tests/lib/rules/no-render-return-value.js invalid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/invalid-007.jsx` | `tests/lib/rules/no-render-return-value.js invalid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/invalid-008.jsx` | `tests/lib/rules/no-render-return-value.js invalid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/valid-000.jsx` | `tests/lib/rules/no-string-refs.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/valid-001.jsx` | `tests/lib/rules/no-string-refs.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/valid-002.jsx` | `tests/lib/rules/no-string-refs.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/valid-003.jsx` | `tests/lib/rules/no-string-refs.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/invalid-000.jsx` | `tests/lib/rules/no-string-refs.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/invalid-001.jsx` | `tests/lib/rules/no-string-refs.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/invalid-002.jsx` | `tests/lib/rules/no-string-refs.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/invalid-003.jsx` | `tests/lib/rules/no-string-refs.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/invalid-004.jsx` | `tests/lib/rules/no-string-refs.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/invalid-005.jsx` | `tests/lib/rules/no-string-refs.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/invalid-006.jsx` | `tests/lib/rules/no-string-refs.js invalid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/valid-000.jsx` | `tests/lib/rules/no-unescaped-entities.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/valid-001.jsx` | `tests/lib/rules/no-unescaped-entities.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/valid-002.jsx` | `tests/lib/rules/no-unescaped-entities.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/valid-003.jsx` | `tests/lib/rules/no-unescaped-entities.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/valid-004.jsx` | `tests/lib/rules/no-unescaped-entities.js valid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/valid-005.jsx` | `tests/lib/rules/no-unescaped-entities.js valid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/valid-006.jsx` | `tests/lib/rules/no-unescaped-entities.js valid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/valid-007.jsx` | `tests/lib/rules/no-unescaped-entities.js valid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/invalid-002.jsx` | `tests/lib/rules/no-unescaped-entities.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/invalid-004.jsx` | `tests/lib/rules/no-unescaped-entities.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/invalid-005.jsx` | `tests/lib/rules/no-unescaped-entities.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/invalid-006.jsx` | `tests/lib/rules/no-unescaped-entities.js invalid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/invalid-007.jsx` | `tests/lib/rules/no-unescaped-entities.js invalid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/valid-000.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/valid-001.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/valid-002.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/valid-003.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/valid-004.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/valid-005.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/valid-006.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/valid-007.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/valid-008.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/valid-009.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/invalid-000.jsx` | `tests/lib/rules/react-in-jsx-scope.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/invalid-001.jsx` | `tests/lib/rules/react-in-jsx-scope.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/invalid-002.jsx` | `tests/lib/rules/react-in-jsx-scope.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/invalid-003.jsx` | `tests/lib/rules/react-in-jsx-scope.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/invalid-004.jsx` | `tests/lib/rules/react-in-jsx-scope.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/invalid-005.jsx` | `tests/lib/rules/react-in-jsx-scope.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/invalid-006.jsx` | `tests/lib/rules/react-in-jsx-scope.js invalid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-000.jsx` | `tests/lib/rules/self-closing-comp.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-001.jsx` | `tests/lib/rules/self-closing-comp.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-002.jsx` | `tests/lib/rules/self-closing-comp.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-003.jsx` | `tests/lib/rules/self-closing-comp.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-004.jsx` | `tests/lib/rules/self-closing-comp.js valid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-005.jsx` | `tests/lib/rules/self-closing-comp.js valid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-006.jsx` | `tests/lib/rules/self-closing-comp.js valid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-007.jsx` | `tests/lib/rules/self-closing-comp.js valid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-008.jsx` | `tests/lib/rules/self-closing-comp.js valid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-009.jsx` | `tests/lib/rules/self-closing-comp.js valid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-010.jsx` | `tests/lib/rules/self-closing-comp.js valid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-011.jsx` | `tests/lib/rules/self-closing-comp.js valid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-012.jsx` | `tests/lib/rules/self-closing-comp.js valid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-013.jsx` | `tests/lib/rules/self-closing-comp.js valid[13]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-014.jsx` | `tests/lib/rules/self-closing-comp.js valid[14]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-015.jsx` | `tests/lib/rules/self-closing-comp.js valid[15]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-016.jsx` | `tests/lib/rules/self-closing-comp.js valid[16]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-017.jsx` | `tests/lib/rules/self-closing-comp.js valid[17]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-018.jsx` | `tests/lib/rules/self-closing-comp.js valid[18]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-019.jsx` | `tests/lib/rules/self-closing-comp.js valid[19]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-020.jsx` | `tests/lib/rules/self-closing-comp.js valid[20]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-021.jsx` | `tests/lib/rules/self-closing-comp.js valid[21]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-022.jsx` | `tests/lib/rules/self-closing-comp.js valid[22]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-023.jsx` | `tests/lib/rules/self-closing-comp.js valid[23]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-024.jsx` | `tests/lib/rules/self-closing-comp.js valid[24]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-025.jsx` | `tests/lib/rules/self-closing-comp.js valid[25]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-026.jsx` | `tests/lib/rules/self-closing-comp.js valid[26]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-027.jsx` | `tests/lib/rules/self-closing-comp.js valid[27]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-028.jsx` | `tests/lib/rules/self-closing-comp.js valid[28]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-029.jsx` | `tests/lib/rules/self-closing-comp.js valid[29]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-030.jsx` | `tests/lib/rules/self-closing-comp.js valid[30]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-031.jsx` | `tests/lib/rules/self-closing-comp.js valid[31]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-032.jsx` | `tests/lib/rules/self-closing-comp.js valid[32]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-033.jsx` | `tests/lib/rules/self-closing-comp.js valid[33]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/valid-034.jsx` | `tests/lib/rules/self-closing-comp.js valid[34]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/invalid-000.jsx` | `tests/lib/rules/self-closing-comp.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/invalid-001.jsx` | `tests/lib/rules/self-closing-comp.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/invalid-002.jsx` | `tests/lib/rules/self-closing-comp.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/invalid-003.jsx` | `tests/lib/rules/self-closing-comp.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/invalid-004.jsx` | `tests/lib/rules/self-closing-comp.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/invalid-005.jsx` | `tests/lib/rules/self-closing-comp.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/invalid-006.jsx` | `tests/lib/rules/self-closing-comp.js invalid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/invalid-007.jsx` | `tests/lib/rules/self-closing-comp.js invalid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/invalid-008.jsx` | `tests/lib/rules/self-closing-comp.js invalid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/invalid-009.jsx` | `tests/lib/rules/self-closing-comp.js invalid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/invalid-010.jsx` | `tests/lib/rules/self-closing-comp.js invalid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/invalid-011.jsx` | `tests/lib/rules/self-closing-comp.js invalid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/valid-000.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/valid-001.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/valid-002.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/valid-003.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/valid-004.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/valid-005.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/valid-006.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/valid-007.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/valid-008.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/valid-009.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/valid-010.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[10]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/valid-011.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[11]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/valid-012.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[12]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/valid-013.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[13]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/invalid-000.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/invalid-001.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/invalid-002.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/invalid-003.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/invalid-004.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[4]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/invalid-005.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[5]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/invalid-006.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[6]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/invalid-007.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[7]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/invalid-008.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[8]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/invalid-009.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[9]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-001.jsx` | `tests/lib/rules/button-has-type.js invalid[1]` | derived (`fixtures/button-has-type/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-002.jsx` | `tests/lib/rules/button-has-type.js invalid[2]` | derived (`fixtures/button-has-type/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-003.jsx` | `tests/lib/rules/button-has-type.js invalid[3]` | derived (`fixtures/button-has-type/invalid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-004.jsx` | `tests/lib/rules/button-has-type.js invalid[4]` | derived (`fixtures/button-has-type/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-005.jsx` | `tests/lib/rules/button-has-type.js invalid[5]` | derived (`fixtures/button-has-type/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-006.jsx` | `tests/lib/rules/button-has-type.js invalid[6]` | derived (`fixtures/button-has-type/invalid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-007.jsx` | `tests/lib/rules/button-has-type.js invalid[7]` | derived (`fixtures/button-has-type/invalid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-008.jsx` | `tests/lib/rules/button-has-type.js invalid[8]` | derived (`fixtures/button-has-type/invalid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-009.jsx` | `tests/lib/rules/button-has-type.js invalid[9]` | derived (`fixtures/button-has-type/invalid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-010.jsx` | `tests/lib/rules/button-has-type.js invalid[10]` | derived (`fixtures/button-has-type/invalid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-011.jsx` | `tests/lib/rules/button-has-type.js invalid[11]` | derived (`fixtures/button-has-type/invalid-011.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-012.jsx` | `tests/lib/rules/button-has-type.js invalid[12]` | derived (`fixtures/button-has-type/invalid-012.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-014.jsx` | `tests/lib/rules/button-has-type.js invalid[14]` | derived (`fixtures/button-has-type/invalid-014.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-015.jsx` | `tests/lib/rules/button-has-type.js invalid[15]` | derived (`fixtures/button-has-type/invalid-015.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-016.jsx` | `tests/lib/rules/button-has-type.js invalid[16]` | derived (`fixtures/button-has-type/invalid-016.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-017.jsx` | `tests/lib/rules/button-has-type.js invalid[17]` | derived (`fixtures/button-has-type/invalid-017.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-018.jsx` | `tests/lib/rules/button-has-type.js invalid[18]` | derived (`fixtures/button-has-type/invalid-018.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-019.jsx` | `tests/lib/rules/button-has-type.js invalid[19]` | derived (`fixtures/button-has-type/invalid-019.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-020.jsx` | `tests/lib/rules/button-has-type.js invalid[20]` | derived (`fixtures/button-has-type/invalid-020.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-021.jsx` | `tests/lib/rules/button-has-type.js invalid[21]` | derived (`fixtures/button-has-type/invalid-021.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-022.jsx` | `tests/lib/rules/button-has-type.js invalid[22]` | derived (`fixtures/button-has-type/invalid-022.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-023.jsx` | `tests/lib/rules/button-has-type.js invalid[23]` | derived (`fixtures/button-has-type/invalid-023.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-024.jsx` | `tests/lib/rules/button-has-type.js invalid[24]` | derived (`fixtures/button-has-type/invalid-024.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-025.jsx` | `tests/lib/rules/button-has-type.js invalid[25]` | derived (`fixtures/button-has-type/invalid-025.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-026.jsx` | `tests/lib/rules/button-has-type.js invalid[26]` | derived (`fixtures/button-has-type/invalid-026.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/invalid-027.jsx` | `tests/lib/rules/button-has-type.js invalid[27]` | derived (`fixtures/button-has-type/invalid-027.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-001.jsx` | `tests/lib/rules/button-has-type.js valid[1]` | derived (`fixtures/button-has-type/valid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-003.jsx` | `tests/lib/rules/button-has-type.js valid[3]` | derived (`fixtures/button-has-type/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-004.jsx` | `tests/lib/rules/button-has-type.js valid[4]` | derived (`fixtures/button-has-type/valid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-008.jsx` | `tests/lib/rules/button-has-type.js valid[8]` | derived (`fixtures/button-has-type/valid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-009.jsx` | `tests/lib/rules/button-has-type.js valid[9]` | derived (`fixtures/button-has-type/valid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-010.jsx` | `tests/lib/rules/button-has-type.js valid[10]` | derived (`fixtures/button-has-type/valid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-012.jsx` | `tests/lib/rules/button-has-type.js valid[12]` | derived (`fixtures/button-has-type/valid-012.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-013.jsx` | `tests/lib/rules/button-has-type.js valid[13]` | derived (`fixtures/button-has-type/valid-013.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-014.jsx` | `tests/lib/rules/button-has-type.js valid[14]` | derived (`fixtures/button-has-type/valid-014.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-015.jsx` | `tests/lib/rules/button-has-type.js valid[15]` | derived (`fixtures/button-has-type/valid-015.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-016.jsx` | `tests/lib/rules/button-has-type.js valid[16]` | derived (`fixtures/button-has-type/valid-016.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-017.jsx` | `tests/lib/rules/button-has-type.js valid[17]` | derived (`fixtures/button-has-type/valid-017.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-018.jsx` | `tests/lib/rules/button-has-type.js valid[18]` | derived (`fixtures/button-has-type/valid-018.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-019.jsx` | `tests/lib/rules/button-has-type.js valid[19]` | derived (`fixtures/button-has-type/valid-019.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-020.jsx` | `tests/lib/rules/button-has-type.js valid[20]` | derived (`fixtures/button-has-type/valid-020.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-021.jsx` | `tests/lib/rules/button-has-type.js valid[21]` | derived (`fixtures/button-has-type/valid-021.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-022.jsx` | `tests/lib/rules/button-has-type.js valid[22]` | derived (`fixtures/button-has-type/valid-022.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-023.jsx` | `tests/lib/rules/button-has-type.js valid[23]` | derived (`fixtures/button-has-type/valid-023.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-024.jsx` | `tests/lib/rules/button-has-type.js valid[24]` | derived (`fixtures/button-has-type/valid-024.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-025.jsx` | `tests/lib/rules/button-has-type.js valid[25]` | derived (`fixtures/button-has-type/valid-025.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-026.jsx` | `tests/lib/rules/button-has-type.js valid[26]` | derived (`fixtures/button-has-type/valid-026.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-027.jsx` | `tests/lib/rules/button-has-type.js valid[27]` | derived (`fixtures/button-has-type/valid-027.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-028.jsx` | `tests/lib/rules/button-has-type.js valid[28]` | derived (`fixtures/button-has-type/valid-028.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/button-has-type/valid-029.tsx` | `tests/lib/rules/button-has-type.js valid[29]` | derived (`fixtures/button-has-type/valid-029.tsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-000.jsx` | `tests/lib/rules/jsx-key.js invalid[0]` | derived (`fixtures/jsx-key/invalid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-001.jsx` | `tests/lib/rules/jsx-key.js invalid[1]` | derived (`fixtures/jsx-key/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-002.jsx` | `tests/lib/rules/jsx-key.js invalid[2]` | derived (`fixtures/jsx-key/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-003.jsx` | `tests/lib/rules/jsx-key.js invalid[3]` | derived (`fixtures/jsx-key/invalid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-004.jsx` | `tests/lib/rules/jsx-key.js invalid[4]` | derived (`fixtures/jsx-key/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-005.jsx` | `tests/lib/rules/jsx-key.js invalid[5]` | derived (`fixtures/jsx-key/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-006.jsx` | `tests/lib/rules/jsx-key.js invalid[6]` | derived (`fixtures/jsx-key/invalid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-007.jsx` | `tests/lib/rules/jsx-key.js invalid[7]` | derived (`fixtures/jsx-key/invalid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-008.jsx` | `tests/lib/rules/jsx-key.js invalid[8]` | derived (`fixtures/jsx-key/invalid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-009.jsx` | `tests/lib/rules/jsx-key.js invalid[9]` | derived (`fixtures/jsx-key/invalid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-010.jsx` | `tests/lib/rules/jsx-key.js invalid[10]` | derived (`fixtures/jsx-key/invalid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-011.jsx` | `tests/lib/rules/jsx-key.js invalid[11]` | derived (`fixtures/jsx-key/invalid-011.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-012.tsx` | `tests/lib/rules/jsx-key.js invalid[12]` | derived (`fixtures/jsx-key/invalid-012.tsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-013.tsx` | `tests/lib/rules/jsx-key.js invalid[13]` | derived (`fixtures/jsx-key/invalid-013.tsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-014.jsx` | `tests/lib/rules/jsx-key.js invalid[14]` | derived (`fixtures/jsx-key/invalid-014.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-016.jsx` | `tests/lib/rules/jsx-key.js invalid[16]` | derived (`fixtures/jsx-key/invalid-016.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-017.jsx` | `tests/lib/rules/jsx-key.js invalid[17]` | derived (`fixtures/jsx-key/invalid-017.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-018.jsx` | `tests/lib/rules/jsx-key.js invalid[18]` | derived (`fixtures/jsx-key/invalid-018.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-019.jsx` | `tests/lib/rules/jsx-key.js invalid[19]` | derived (`fixtures/jsx-key/invalid-019.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-020.jsx` | `tests/lib/rules/jsx-key.js invalid[20]` | derived (`fixtures/jsx-key/invalid-020.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-021.jsx` | `tests/lib/rules/jsx-key.js invalid[21]` | derived (`fixtures/jsx-key/invalid-021.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-022.jsx` | `tests/lib/rules/jsx-key.js invalid[22]` | derived (`fixtures/jsx-key/invalid-022.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/invalid-023.jsx` | `tests/lib/rules/jsx-key.js invalid[23]` | derived (`fixtures/jsx-key/invalid-023.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-000.jsx` | `tests/lib/rules/jsx-key.js valid[0]` | derived (`fixtures/jsx-key/valid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-001.jsx` | `tests/lib/rules/jsx-key.js valid[1]` | derived (`fixtures/jsx-key/valid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-002.jsx` | `tests/lib/rules/jsx-key.js valid[2]` | derived (`fixtures/jsx-key/valid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-003.jsx` | `tests/lib/rules/jsx-key.js valid[3]` | derived (`fixtures/jsx-key/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-004.jsx` | `tests/lib/rules/jsx-key.js valid[4]` | derived (`fixtures/jsx-key/valid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-005.jsx` | `tests/lib/rules/jsx-key.js valid[5]` | derived (`fixtures/jsx-key/valid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-006.jsx` | `tests/lib/rules/jsx-key.js valid[6]` | derived (`fixtures/jsx-key/valid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-007.jsx` | `tests/lib/rules/jsx-key.js valid[7]` | derived (`fixtures/jsx-key/valid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-008.jsx` | `tests/lib/rules/jsx-key.js valid[8]` | derived (`fixtures/jsx-key/valid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-009.jsx` | `tests/lib/rules/jsx-key.js valid[9]` | derived (`fixtures/jsx-key/valid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-010.jsx` | `tests/lib/rules/jsx-key.js valid[10]` | derived (`fixtures/jsx-key/valid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-011.jsx` | `tests/lib/rules/jsx-key.js valid[11]` | derived (`fixtures/jsx-key/valid-011.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-012.jsx` | `tests/lib/rules/jsx-key.js valid[12]` | derived (`fixtures/jsx-key/valid-012.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-013.jsx` | `tests/lib/rules/jsx-key.js valid[13]` | derived (`fixtures/jsx-key/valid-013.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-014.jsx` | `tests/lib/rules/jsx-key.js valid[14]` | derived (`fixtures/jsx-key/valid-014.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-015.jsx` | `tests/lib/rules/jsx-key.js valid[15]` | derived (`fixtures/jsx-key/valid-015.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-016.jsx` | `tests/lib/rules/jsx-key.js valid[16]` | derived (`fixtures/jsx-key/valid-016.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-017.jsx` | `tests/lib/rules/jsx-key.js valid[17]` | derived (`fixtures/jsx-key/valid-017.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-018.jsx` | `tests/lib/rules/jsx-key.js valid[18]` | derived (`fixtures/jsx-key/valid-018.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-020.jsx` | `tests/lib/rules/jsx-key.js valid[20]` | derived (`fixtures/jsx-key/valid-020.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-021.jsx` | `tests/lib/rules/jsx-key.js valid[21]` | derived (`fixtures/jsx-key/valid-021.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-022.jsx` | `tests/lib/rules/jsx-key.js valid[22]` | derived (`fixtures/jsx-key/valid-022.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-023.jsx` | `tests/lib/rules/jsx-key.js valid[23]` | derived (`fixtures/jsx-key/valid-023.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-024.jsx` | `tests/lib/rules/jsx-key.js valid[24]` | derived (`fixtures/jsx-key/valid-024.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-025.tsx` | `tests/lib/rules/jsx-key.js valid[25]` | derived (`fixtures/jsx-key/valid-025.tsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-026.jsx` | `tests/lib/rules/jsx-key.js valid[26]` | derived (`fixtures/jsx-key/valid-026.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-027.jsx` | `tests/lib/rules/jsx-key.js valid[27]` | derived (`fixtures/jsx-key/valid-027.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-028.jsx` | `tests/lib/rules/jsx-key.js valid[28]` | derived (`fixtures/jsx-key/valid-028.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-029.tsx` | `tests/lib/rules/jsx-key.js valid[29]` | derived (`fixtures/jsx-key/valid-029.tsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-030.jsx` | `tests/lib/rules/jsx-key.js valid[30]` | derived (`fixtures/jsx-key/valid-030.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-031.jsx` | `tests/lib/rules/jsx-key.js valid[31]` | derived (`fixtures/jsx-key/valid-031.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-key/valid-032.jsx` | `tests/lib/rules/jsx-key.js valid[32]` | derived (`fixtures/jsx-key/valid-032.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/invalid-000.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js invalid[0]` | derived (`fixtures/jsx-no-comment-textnodes/invalid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/invalid-001.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js invalid[1]` | derived (`fixtures/jsx-no-comment-textnodes/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/invalid-002.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js invalid[2]` | derived (`fixtures/jsx-no-comment-textnodes/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/invalid-003.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js invalid[3]` | derived (`fixtures/jsx-no-comment-textnodes/invalid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/invalid-004.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js invalid[4]` | derived (`fixtures/jsx-no-comment-textnodes/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/invalid-005.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js invalid[5]` | derived (`fixtures/jsx-no-comment-textnodes/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/invalid-006.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js invalid[6]` | derived (`fixtures/jsx-no-comment-textnodes/invalid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/valid-000.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[0]` | derived (`fixtures/jsx-no-comment-textnodes/valid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/valid-001.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[1]` | derived (`fixtures/jsx-no-comment-textnodes/valid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/valid-002.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[2]` | derived (`fixtures/jsx-no-comment-textnodes/valid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/valid-003.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[3]` | derived (`fixtures/jsx-no-comment-textnodes/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/valid-004.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[4]` | derived (`fixtures/jsx-no-comment-textnodes/valid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/valid-005.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[5]` | derived (`fixtures/jsx-no-comment-textnodes/valid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/valid-006.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[6]` | derived (`fixtures/jsx-no-comment-textnodes/valid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/valid-007.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[7]` | derived (`fixtures/jsx-no-comment-textnodes/valid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/valid-008.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[8]` | derived (`fixtures/jsx-no-comment-textnodes/valid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/valid-010.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[10]` | derived (`fixtures/jsx-no-comment-textnodes/valid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/valid-012.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[12]` | derived (`fixtures/jsx-no-comment-textnodes/valid-012.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/valid-013.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[13]` | derived (`fixtures/jsx-no-comment-textnodes/valid-013.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-comment-textnodes/valid-016.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js valid[16]` | derived (`fixtures/jsx-no-comment-textnodes/valid-016.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/invalid-000.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js invalid[0]` | derived (`fixtures/jsx-no-duplicate-props/invalid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/invalid-001.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js invalid[1]` | derived (`fixtures/jsx-no-duplicate-props/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/invalid-002.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js invalid[2]` | derived (`fixtures/jsx-no-duplicate-props/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/invalid-003.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js invalid[3]` | derived (`fixtures/jsx-no-duplicate-props/invalid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/invalid-004.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js invalid[4]` | derived (`fixtures/jsx-no-duplicate-props/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/invalid-005.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js invalid[5]` | derived (`fixtures/jsx-no-duplicate-props/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/valid-000.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[0]` | derived (`fixtures/jsx-no-duplicate-props/valid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/valid-001.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[1]` | derived (`fixtures/jsx-no-duplicate-props/valid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/valid-002.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[2]` | derived (`fixtures/jsx-no-duplicate-props/valid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/valid-003.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[3]` | derived (`fixtures/jsx-no-duplicate-props/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/valid-004.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[4]` | derived (`fixtures/jsx-no-duplicate-props/valid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/valid-005.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[5]` | derived (`fixtures/jsx-no-duplicate-props/valid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/valid-006.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[6]` | derived (`fixtures/jsx-no-duplicate-props/valid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/valid-007.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[7]` | derived (`fixtures/jsx-no-duplicate-props/valid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/valid-008.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[8]` | derived (`fixtures/jsx-no-duplicate-props/valid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/valid-009.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[9]` | derived (`fixtures/jsx-no-duplicate-props/valid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/valid-010.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[10]` | derived (`fixtures/jsx-no-duplicate-props/valid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/valid-011.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[11]` | derived (`fixtures/jsx-no-duplicate-props/valid-011.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-duplicate-props/valid-012.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js valid[12]` | derived (`fixtures/jsx-no-duplicate-props/valid-012.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-000.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[0]` | derived (`fixtures/jsx-no-target-blank/invalid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-001.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[1]` | derived (`fixtures/jsx-no-target-blank/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-002.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[2]` | derived (`fixtures/jsx-no-target-blank/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-003.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[3]` | derived (`fixtures/jsx-no-target-blank/invalid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-004.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[4]` | derived (`fixtures/jsx-no-target-blank/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-005.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[5]` | derived (`fixtures/jsx-no-target-blank/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-006.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[6]` | derived (`fixtures/jsx-no-target-blank/invalid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-007.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[7]` | derived (`fixtures/jsx-no-target-blank/invalid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-008.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[8]` | derived (`fixtures/jsx-no-target-blank/invalid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-009.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[9]` | derived (`fixtures/jsx-no-target-blank/invalid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-010.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[10]` | derived (`fixtures/jsx-no-target-blank/invalid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-011.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[11]` | derived (`fixtures/jsx-no-target-blank/invalid-011.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-012.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[12]` | derived (`fixtures/jsx-no-target-blank/invalid-012.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-013.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[13]` | derived (`fixtures/jsx-no-target-blank/invalid-013.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-014.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[14]` | derived (`fixtures/jsx-no-target-blank/invalid-014.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-015.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[15]` | derived (`fixtures/jsx-no-target-blank/invalid-015.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-016.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[16]` | derived (`fixtures/jsx-no-target-blank/invalid-016.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-017.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[17]` | derived (`fixtures/jsx-no-target-blank/invalid-017.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-018.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[18]` | derived (`fixtures/jsx-no-target-blank/invalid-018.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-019.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[19]` | derived (`fixtures/jsx-no-target-blank/invalid-019.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-020.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[20]` | derived (`fixtures/jsx-no-target-blank/invalid-020.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-021.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[21]` | derived (`fixtures/jsx-no-target-blank/invalid-021.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-022.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[22]` | derived (`fixtures/jsx-no-target-blank/invalid-022.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-023.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[23]` | derived (`fixtures/jsx-no-target-blank/invalid-023.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-024.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[24]` | derived (`fixtures/jsx-no-target-blank/invalid-024.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-025.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[25]` | derived (`fixtures/jsx-no-target-blank/invalid-025.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-026.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[26]` | derived (`fixtures/jsx-no-target-blank/invalid-026.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-027.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[27]` | derived (`fixtures/jsx-no-target-blank/invalid-027.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-028.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[28]` | derived (`fixtures/jsx-no-target-blank/invalid-028.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-029.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[29]` | derived (`fixtures/jsx-no-target-blank/invalid-029.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-030.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[30]` | derived (`fixtures/jsx-no-target-blank/invalid-030.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-031.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[31]` | derived (`fixtures/jsx-no-target-blank/invalid-031.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-032.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[32]` | derived (`fixtures/jsx-no-target-blank/invalid-032.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-033.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[33]` | derived (`fixtures/jsx-no-target-blank/invalid-033.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-034.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[34]` | derived (`fixtures/jsx-no-target-blank/invalid-034.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-035.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[35]` | derived (`fixtures/jsx-no-target-blank/invalid-035.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-036.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[36]` | derived (`fixtures/jsx-no-target-blank/invalid-036.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-037.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[37]` | derived (`fixtures/jsx-no-target-blank/invalid-037.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-038.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[38]` | derived (`fixtures/jsx-no-target-blank/invalid-038.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-039.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[39]` | derived (`fixtures/jsx-no-target-blank/invalid-039.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-040.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[40]` | derived (`fixtures/jsx-no-target-blank/invalid-040.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-041.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[41]` | derived (`fixtures/jsx-no-target-blank/invalid-041.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-042.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[42]` | derived (`fixtures/jsx-no-target-blank/invalid-042.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-043.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[43]` | derived (`fixtures/jsx-no-target-blank/invalid-043.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-044.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[44]` | derived (`fixtures/jsx-no-target-blank/invalid-044.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-045.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[45]` | derived (`fixtures/jsx-no-target-blank/invalid-045.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-046.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[46]` | derived (`fixtures/jsx-no-target-blank/invalid-046.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-047.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[47]` | derived (`fixtures/jsx-no-target-blank/invalid-047.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-048.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[48]` | derived (`fixtures/jsx-no-target-blank/invalid-048.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/invalid-049.jsx` | `tests/lib/rules/jsx-no-target-blank.js invalid[49]` | derived (`fixtures/jsx-no-target-blank/invalid-049.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-000.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[0]` | derived (`fixtures/jsx-no-target-blank/valid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-003.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[3]` | derived (`fixtures/jsx-no-target-blank/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-004.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[4]` | derived (`fixtures/jsx-no-target-blank/valid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-005.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[5]` | derived (`fixtures/jsx-no-target-blank/valid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-006.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[6]` | derived (`fixtures/jsx-no-target-blank/valid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-007.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[7]` | derived (`fixtures/jsx-no-target-blank/valid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-008.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[8]` | derived (`fixtures/jsx-no-target-blank/valid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-009.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[9]` | derived (`fixtures/jsx-no-target-blank/valid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-010.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[10]` | derived (`fixtures/jsx-no-target-blank/valid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-011.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[11]` | derived (`fixtures/jsx-no-target-blank/valid-011.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-012.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[12]` | derived (`fixtures/jsx-no-target-blank/valid-012.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-013.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[13]` | derived (`fixtures/jsx-no-target-blank/valid-013.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-014.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[14]` | derived (`fixtures/jsx-no-target-blank/valid-014.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-015.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[15]` | derived (`fixtures/jsx-no-target-blank/valid-015.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-016.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[16]` | derived (`fixtures/jsx-no-target-blank/valid-016.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-017.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[17]` | derived (`fixtures/jsx-no-target-blank/valid-017.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-018.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[18]` | derived (`fixtures/jsx-no-target-blank/valid-018.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-019.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[19]` | derived (`fixtures/jsx-no-target-blank/valid-019.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-020.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[20]` | derived (`fixtures/jsx-no-target-blank/valid-020.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-021.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[21]` | derived (`fixtures/jsx-no-target-blank/valid-021.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-022.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[22]` | derived (`fixtures/jsx-no-target-blank/valid-022.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-023.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[23]` | derived (`fixtures/jsx-no-target-blank/valid-023.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-024.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[24]` | derived (`fixtures/jsx-no-target-blank/valid-024.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-025.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[25]` | derived (`fixtures/jsx-no-target-blank/valid-025.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-026.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[26]` | derived (`fixtures/jsx-no-target-blank/valid-026.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-027.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[27]` | derived (`fixtures/jsx-no-target-blank/valid-027.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-028.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[28]` | derived (`fixtures/jsx-no-target-blank/valid-028.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-029.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[29]` | derived (`fixtures/jsx-no-target-blank/valid-029.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-030.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[30]` | derived (`fixtures/jsx-no-target-blank/valid-030.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-031.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[31]` | derived (`fixtures/jsx-no-target-blank/valid-031.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-032.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[32]` | derived (`fixtures/jsx-no-target-blank/valid-032.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-033.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[33]` | derived (`fixtures/jsx-no-target-blank/valid-033.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-034.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[34]` | derived (`fixtures/jsx-no-target-blank/valid-034.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-035.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[35]` | derived (`fixtures/jsx-no-target-blank/valid-035.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-036.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[36]` | derived (`fixtures/jsx-no-target-blank/valid-036.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-037.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[37]` | derived (`fixtures/jsx-no-target-blank/valid-037.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-038.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[38]` | derived (`fixtures/jsx-no-target-blank/valid-038.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-039.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[39]` | derived (`fixtures/jsx-no-target-blank/valid-039.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-040.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[40]` | derived (`fixtures/jsx-no-target-blank/valid-040.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-041.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[41]` | derived (`fixtures/jsx-no-target-blank/valid-041.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-042.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[42]` | derived (`fixtures/jsx-no-target-blank/valid-042.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-043.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[43]` | derived (`fixtures/jsx-no-target-blank/valid-043.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-044.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[44]` | derived (`fixtures/jsx-no-target-blank/valid-044.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-045.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[45]` | derived (`fixtures/jsx-no-target-blank/valid-045.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-046.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[46]` | derived (`fixtures/jsx-no-target-blank/valid-046.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-047.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[47]` | derived (`fixtures/jsx-no-target-blank/valid-047.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-048.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[48]` | derived (`fixtures/jsx-no-target-blank/valid-048.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-049.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[49]` | derived (`fixtures/jsx-no-target-blank/valid-049.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-050.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[50]` | derived (`fixtures/jsx-no-target-blank/valid-050.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-051.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[51]` | derived (`fixtures/jsx-no-target-blank/valid-051.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-052.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[52]` | derived (`fixtures/jsx-no-target-blank/valid-052.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-053.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[53]` | derived (`fixtures/jsx-no-target-blank/valid-053.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-054.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[54]` | derived (`fixtures/jsx-no-target-blank/valid-054.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-055.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[55]` | derived (`fixtures/jsx-no-target-blank/valid-055.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-056.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[56]` | derived (`fixtures/jsx-no-target-blank/valid-056.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-057.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[57]` | derived (`fixtures/jsx-no-target-blank/valid-057.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-058.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[58]` | derived (`fixtures/jsx-no-target-blank/valid-058.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-059.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[59]` | derived (`fixtures/jsx-no-target-blank/valid-059.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-060.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[60]` | derived (`fixtures/jsx-no-target-blank/valid-060.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-target-blank/valid-062.jsx` | `tests/lib/rules/jsx-no-target-blank.js valid[62]` | derived (`fixtures/jsx-no-target-blank/valid-062.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-undef/invalid-000.jsx` | `tests/lib/rules/jsx-no-undef.js invalid[0]` | derived (`fixtures/jsx-no-undef/invalid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-undef/invalid-001.jsx` | `tests/lib/rules/jsx-no-undef.js invalid[1]` | derived (`fixtures/jsx-no-undef/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-undef/invalid-002.jsx` | `tests/lib/rules/jsx-no-undef.js invalid[2]` | derived (`fixtures/jsx-no-undef/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-undef/invalid-003.jsx` | `tests/lib/rules/jsx-no-undef.js invalid[3]` | derived (`fixtures/jsx-no-undef/invalid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-undef/invalid-004.jsx` | `tests/lib/rules/jsx-no-undef.js invalid[4]` | derived (`fixtures/jsx-no-undef/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-undef/invalid-005.jsx` | `tests/lib/rules/jsx-no-undef.js invalid[5]` | derived (`fixtures/jsx-no-undef/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-undef/valid-000.jsx` | `tests/lib/rules/jsx-no-undef.js valid[0]` | derived (`fixtures/jsx-no-undef/valid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-undef/valid-003.jsx` | `tests/lib/rules/jsx-no-undef.js valid[3]` | derived (`fixtures/jsx-no-undef/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-undef/valid-004.jsx` | `tests/lib/rules/jsx-no-undef.js valid[4]` | derived (`fixtures/jsx-no-undef/valid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-undef/valid-005.jsx` | `tests/lib/rules/jsx-no-undef.js valid[5]` | derived (`fixtures/jsx-no-undef/valid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-undef/valid-006.jsx` | `tests/lib/rules/jsx-no-undef.js valid[6]` | derived (`fixtures/jsx-no-undef/valid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-undef/valid-007.jsx` | `tests/lib/rules/jsx-no-undef.js valid[7]` | derived (`fixtures/jsx-no-undef/valid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/jsx-no-undef/valid-008.jsx` | `tests/lib/rules/jsx-no-undef.js valid[8]` | derived (`fixtures/jsx-no-undef/valid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-000.jsx` | `tests/lib/rules/no-array-index-key.js invalid[0]` | derived (`fixtures/no-array-index-key/invalid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-001.jsx` | `tests/lib/rules/no-array-index-key.js invalid[1]` | derived (`fixtures/no-array-index-key/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-002.jsx` | `tests/lib/rules/no-array-index-key.js invalid[2]` | derived (`fixtures/no-array-index-key/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-003.jsx` | `tests/lib/rules/no-array-index-key.js invalid[3]` | derived (`fixtures/no-array-index-key/invalid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-004.jsx` | `tests/lib/rules/no-array-index-key.js invalid[4]` | derived (`fixtures/no-array-index-key/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-005.jsx` | `tests/lib/rules/no-array-index-key.js invalid[5]` | derived (`fixtures/no-array-index-key/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-006.jsx` | `tests/lib/rules/no-array-index-key.js invalid[6]` | derived (`fixtures/no-array-index-key/invalid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-007.jsx` | `tests/lib/rules/no-array-index-key.js invalid[7]` | derived (`fixtures/no-array-index-key/invalid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-008.jsx` | `tests/lib/rules/no-array-index-key.js invalid[8]` | derived (`fixtures/no-array-index-key/invalid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-009.jsx` | `tests/lib/rules/no-array-index-key.js invalid[9]` | derived (`fixtures/no-array-index-key/invalid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-010.jsx` | `tests/lib/rules/no-array-index-key.js invalid[10]` | derived (`fixtures/no-array-index-key/invalid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-011.jsx` | `tests/lib/rules/no-array-index-key.js invalid[11]` | derived (`fixtures/no-array-index-key/invalid-011.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-012.jsx` | `tests/lib/rules/no-array-index-key.js invalid[12]` | derived (`fixtures/no-array-index-key/invalid-012.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-013.jsx` | `tests/lib/rules/no-array-index-key.js invalid[13]` | derived (`fixtures/no-array-index-key/invalid-013.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-014.jsx` | `tests/lib/rules/no-array-index-key.js invalid[14]` | derived (`fixtures/no-array-index-key/invalid-014.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-015.jsx` | `tests/lib/rules/no-array-index-key.js invalid[15]` | derived (`fixtures/no-array-index-key/invalid-015.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-016.jsx` | `tests/lib/rules/no-array-index-key.js invalid[16]` | derived (`fixtures/no-array-index-key/invalid-016.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-017.jsx` | `tests/lib/rules/no-array-index-key.js invalid[17]` | derived (`fixtures/no-array-index-key/invalid-017.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-018.jsx` | `tests/lib/rules/no-array-index-key.js invalid[18]` | derived (`fixtures/no-array-index-key/invalid-018.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-019.jsx` | `tests/lib/rules/no-array-index-key.js invalid[19]` | derived (`fixtures/no-array-index-key/invalid-019.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-020.jsx` | `tests/lib/rules/no-array-index-key.js invalid[20]` | derived (`fixtures/no-array-index-key/invalid-020.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-021.jsx` | `tests/lib/rules/no-array-index-key.js invalid[21]` | derived (`fixtures/no-array-index-key/invalid-021.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-022.jsx` | `tests/lib/rules/no-array-index-key.js invalid[22]` | derived (`fixtures/no-array-index-key/invalid-022.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-023.jsx` | `tests/lib/rules/no-array-index-key.js invalid[23]` | derived (`fixtures/no-array-index-key/invalid-023.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-024.jsx` | `tests/lib/rules/no-array-index-key.js invalid[24]` | derived (`fixtures/no-array-index-key/invalid-024.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-025.jsx` | `tests/lib/rules/no-array-index-key.js invalid[25]` | derived (`fixtures/no-array-index-key/invalid-025.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-026.jsx` | `tests/lib/rules/no-array-index-key.js invalid[26]` | derived (`fixtures/no-array-index-key/invalid-026.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-027.jsx` | `tests/lib/rules/no-array-index-key.js invalid[27]` | derived (`fixtures/no-array-index-key/invalid-027.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-028.jsx` | `tests/lib/rules/no-array-index-key.js invalid[28]` | derived (`fixtures/no-array-index-key/invalid-028.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-029.jsx` | `tests/lib/rules/no-array-index-key.js invalid[29]` | derived (`fixtures/no-array-index-key/invalid-029.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-030.jsx` | `tests/lib/rules/no-array-index-key.js invalid[30]` | derived (`fixtures/no-array-index-key/invalid-030.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-031.jsx` | `tests/lib/rules/no-array-index-key.js invalid[31]` | derived (`fixtures/no-array-index-key/invalid-031.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-032.jsx` | `tests/lib/rules/no-array-index-key.js invalid[32]` | derived (`fixtures/no-array-index-key/invalid-032.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-033.jsx` | `tests/lib/rules/no-array-index-key.js invalid[33]` | derived (`fixtures/no-array-index-key/invalid-033.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-034.jsx` | `tests/lib/rules/no-array-index-key.js invalid[34]` | derived (`fixtures/no-array-index-key/invalid-034.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-035.jsx` | `tests/lib/rules/no-array-index-key.js invalid[35]` | derived (`fixtures/no-array-index-key/invalid-035.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-036.jsx` | `tests/lib/rules/no-array-index-key.js invalid[36]` | derived (`fixtures/no-array-index-key/invalid-036.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-037.jsx` | `tests/lib/rules/no-array-index-key.js invalid[37]` | derived (`fixtures/no-array-index-key/invalid-037.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-038.jsx` | `tests/lib/rules/no-array-index-key.js invalid[38]` | derived (`fixtures/no-array-index-key/invalid-038.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-039.jsx` | `tests/lib/rules/no-array-index-key.js invalid[39]` | derived (`fixtures/no-array-index-key/invalid-039.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/invalid-040.jsx` | `tests/lib/rules/no-array-index-key.js invalid[40]` | derived (`fixtures/no-array-index-key/invalid-040.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-000.jsx` | `tests/lib/rules/no-array-index-key.js valid[0]` | derived (`fixtures/no-array-index-key/valid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-001.jsx` | `tests/lib/rules/no-array-index-key.js valid[1]` | derived (`fixtures/no-array-index-key/valid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-002.jsx` | `tests/lib/rules/no-array-index-key.js valid[2]` | derived (`fixtures/no-array-index-key/valid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-003.jsx` | `tests/lib/rules/no-array-index-key.js valid[3]` | derived (`fixtures/no-array-index-key/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-004.jsx` | `tests/lib/rules/no-array-index-key.js valid[4]` | derived (`fixtures/no-array-index-key/valid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-005.jsx` | `tests/lib/rules/no-array-index-key.js valid[5]` | derived (`fixtures/no-array-index-key/valid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-006.jsx` | `tests/lib/rules/no-array-index-key.js valid[6]` | derived (`fixtures/no-array-index-key/valid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-007.jsx` | `tests/lib/rules/no-array-index-key.js valid[7]` | derived (`fixtures/no-array-index-key/valid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-008.jsx` | `tests/lib/rules/no-array-index-key.js valid[8]` | derived (`fixtures/no-array-index-key/valid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-009.jsx` | `tests/lib/rules/no-array-index-key.js valid[9]` | derived (`fixtures/no-array-index-key/valid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-010.jsx` | `tests/lib/rules/no-array-index-key.js valid[10]` | derived (`fixtures/no-array-index-key/valid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-011.jsx` | `tests/lib/rules/no-array-index-key.js valid[11]` | derived (`fixtures/no-array-index-key/valid-011.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-012.jsx` | `tests/lib/rules/no-array-index-key.js valid[12]` | derived (`fixtures/no-array-index-key/valid-012.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-013.jsx` | `tests/lib/rules/no-array-index-key.js valid[13]` | derived (`fixtures/no-array-index-key/valid-013.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-014.jsx` | `tests/lib/rules/no-array-index-key.js valid[14]` | derived (`fixtures/no-array-index-key/valid-014.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-015.jsx` | `tests/lib/rules/no-array-index-key.js valid[15]` | derived (`fixtures/no-array-index-key/valid-015.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-016.jsx` | `tests/lib/rules/no-array-index-key.js valid[16]` | derived (`fixtures/no-array-index-key/valid-016.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-017.jsx` | `tests/lib/rules/no-array-index-key.js valid[17]` | derived (`fixtures/no-array-index-key/valid-017.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-018.jsx` | `tests/lib/rules/no-array-index-key.js valid[18]` | derived (`fixtures/no-array-index-key/valid-018.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-019.jsx` | `tests/lib/rules/no-array-index-key.js valid[19]` | derived (`fixtures/no-array-index-key/valid-019.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-020.jsx` | `tests/lib/rules/no-array-index-key.js valid[20]` | derived (`fixtures/no-array-index-key/valid-020.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-021.jsx` | `tests/lib/rules/no-array-index-key.js valid[21]` | derived (`fixtures/no-array-index-key/valid-021.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-022.jsx` | `tests/lib/rules/no-array-index-key.js valid[22]` | derived (`fixtures/no-array-index-key/valid-022.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-023.jsx` | `tests/lib/rules/no-array-index-key.js valid[23]` | derived (`fixtures/no-array-index-key/valid-023.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-024.jsx` | `tests/lib/rules/no-array-index-key.js valid[24]` | derived (`fixtures/no-array-index-key/valid-024.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-025.jsx` | `tests/lib/rules/no-array-index-key.js valid[25]` | derived (`fixtures/no-array-index-key/valid-025.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-026.jsx` | `tests/lib/rules/no-array-index-key.js valid[26]` | derived (`fixtures/no-array-index-key/valid-026.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-027.jsx` | `tests/lib/rules/no-array-index-key.js valid[27]` | derived (`fixtures/no-array-index-key/valid-027.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-028.jsx` | `tests/lib/rules/no-array-index-key.js valid[28]` | derived (`fixtures/no-array-index-key/valid-028.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-array-index-key/valid-029.jsx` | `tests/lib/rules/no-array-index-key.js valid[29]` | derived (`fixtures/no-array-index-key/valid-029.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-001.jsx` | `tests/lib/rules/no-children-prop.js invalid[1]` | derived (`fixtures/no-children-prop/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-004.jsx` | `tests/lib/rules/no-children-prop.js invalid[4]` | derived (`fixtures/no-children-prop/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-005.jsx` | `tests/lib/rules/no-children-prop.js invalid[5]` | derived (`fixtures/no-children-prop/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-006.jsx` | `tests/lib/rules/no-children-prop.js invalid[6]` | derived (`fixtures/no-children-prop/invalid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-007.jsx` | `tests/lib/rules/no-children-prop.js invalid[7]` | derived (`fixtures/no-children-prop/invalid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-008.jsx` | `tests/lib/rules/no-children-prop.js invalid[8]` | derived (`fixtures/no-children-prop/invalid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-009.jsx` | `tests/lib/rules/no-children-prop.js invalid[9]` | derived (`fixtures/no-children-prop/invalid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-010.jsx` | `tests/lib/rules/no-children-prop.js invalid[10]` | derived (`fixtures/no-children-prop/invalid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-011.jsx` | `tests/lib/rules/no-children-prop.js invalid[11]` | derived (`fixtures/no-children-prop/invalid-011.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-012.jsx` | `tests/lib/rules/no-children-prop.js invalid[12]` | derived (`fixtures/no-children-prop/invalid-012.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-013.jsx` | `tests/lib/rules/no-children-prop.js invalid[13]` | derived (`fixtures/no-children-prop/invalid-013.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-014.jsx` | `tests/lib/rules/no-children-prop.js invalid[14]` | derived (`fixtures/no-children-prop/invalid-014.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-015.jsx` | `tests/lib/rules/no-children-prop.js invalid[15]` | derived (`fixtures/no-children-prop/invalid-015.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-016.jsx` | `tests/lib/rules/no-children-prop.js invalid[16]` | derived (`fixtures/no-children-prop/invalid-016.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-017.jsx` | `tests/lib/rules/no-children-prop.js invalid[17]` | derived (`fixtures/no-children-prop/invalid-017.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-018.jsx` | `tests/lib/rules/no-children-prop.js invalid[18]` | derived (`fixtures/no-children-prop/invalid-018.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-019.jsx` | `tests/lib/rules/no-children-prop.js invalid[19]` | derived (`fixtures/no-children-prop/invalid-019.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-020.jsx` | `tests/lib/rules/no-children-prop.js invalid[20]` | derived (`fixtures/no-children-prop/invalid-020.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-021.jsx` | `tests/lib/rules/no-children-prop.js invalid[21]` | derived (`fixtures/no-children-prop/invalid-021.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/invalid-022.jsx` | `tests/lib/rules/no-children-prop.js invalid[22]` | derived (`fixtures/no-children-prop/invalid-022.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-002.jsx` | `tests/lib/rules/no-children-prop.js valid[2]` | derived (`fixtures/no-children-prop/valid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-003.jsx` | `tests/lib/rules/no-children-prop.js valid[3]` | derived (`fixtures/no-children-prop/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-005.jsx` | `tests/lib/rules/no-children-prop.js valid[5]` | derived (`fixtures/no-children-prop/valid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-007.jsx` | `tests/lib/rules/no-children-prop.js valid[7]` | derived (`fixtures/no-children-prop/valid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-008.jsx` | `tests/lib/rules/no-children-prop.js valid[8]` | derived (`fixtures/no-children-prop/valid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-009.jsx` | `tests/lib/rules/no-children-prop.js valid[9]` | derived (`fixtures/no-children-prop/valid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-011.jsx` | `tests/lib/rules/no-children-prop.js valid[11]` | derived (`fixtures/no-children-prop/valid-011.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-013.jsx` | `tests/lib/rules/no-children-prop.js valid[13]` | derived (`fixtures/no-children-prop/valid-013.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-014.jsx` | `tests/lib/rules/no-children-prop.js valid[14]` | derived (`fixtures/no-children-prop/valid-014.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-015.jsx` | `tests/lib/rules/no-children-prop.js valid[15]` | derived (`fixtures/no-children-prop/valid-015.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-017.jsx` | `tests/lib/rules/no-children-prop.js valid[17]` | derived (`fixtures/no-children-prop/valid-017.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-018.jsx` | `tests/lib/rules/no-children-prop.js valid[18]` | derived (`fixtures/no-children-prop/valid-018.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-019.jsx` | `tests/lib/rules/no-children-prop.js valid[19]` | derived (`fixtures/no-children-prop/valid-019.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-020.jsx` | `tests/lib/rules/no-children-prop.js valid[20]` | derived (`fixtures/no-children-prop/valid-020.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-021.jsx` | `tests/lib/rules/no-children-prop.js valid[21]` | derived (`fixtures/no-children-prop/valid-021.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-022.jsx` | `tests/lib/rules/no-children-prop.js valid[22]` | derived (`fixtures/no-children-prop/valid-022.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-023.jsx` | `tests/lib/rules/no-children-prop.js valid[23]` | derived (`fixtures/no-children-prop/valid-023.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-024.jsx` | `tests/lib/rules/no-children-prop.js valid[24]` | derived (`fixtures/no-children-prop/valid-024.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-025.jsx` | `tests/lib/rules/no-children-prop.js valid[25]` | derived (`fixtures/no-children-prop/valid-025.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-026.jsx` | `tests/lib/rules/no-children-prop.js valid[26]` | derived (`fixtures/no-children-prop/valid-026.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-027.jsx` | `tests/lib/rules/no-children-prop.js valid[27]` | derived (`fixtures/no-children-prop/valid-027.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-028.jsx` | `tests/lib/rules/no-children-prop.js valid[28]` | derived (`fixtures/no-children-prop/valid-028.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-029.jsx` | `tests/lib/rules/no-children-prop.js valid[29]` | derived (`fixtures/no-children-prop/valid-029.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-030.jsx` | `tests/lib/rules/no-children-prop.js valid[30]` | derived (`fixtures/no-children-prop/valid-030.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-031.jsx` | `tests/lib/rules/no-children-prop.js valid[31]` | derived (`fixtures/no-children-prop/valid-031.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-032.jsx` | `tests/lib/rules/no-children-prop.js valid[32]` | derived (`fixtures/no-children-prop/valid-032.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-033.jsx` | `tests/lib/rules/no-children-prop.js valid[33]` | derived (`fixtures/no-children-prop/valid-033.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-034.jsx` | `tests/lib/rules/no-children-prop.js valid[34]` | derived (`fixtures/no-children-prop/valid-034.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-035.jsx` | `tests/lib/rules/no-children-prop.js valid[35]` | derived (`fixtures/no-children-prop/valid-035.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-036.jsx` | `tests/lib/rules/no-children-prop.js valid[36]` | derived (`fixtures/no-children-prop/valid-036.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-037.jsx` | `tests/lib/rules/no-children-prop.js valid[37]` | derived (`fixtures/no-children-prop/valid-037.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-038.jsx` | `tests/lib/rules/no-children-prop.js valid[38]` | derived (`fixtures/no-children-prop/valid-038.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-039.jsx` | `tests/lib/rules/no-children-prop.js valid[39]` | derived (`fixtures/no-children-prop/valid-039.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-040.jsx` | `tests/lib/rules/no-children-prop.js valid[40]` | derived (`fixtures/no-children-prop/valid-040.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-041.jsx` | `tests/lib/rules/no-children-prop.js valid[41]` | derived (`fixtures/no-children-prop/valid-041.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-042.jsx` | `tests/lib/rules/no-children-prop.js valid[42]` | derived (`fixtures/no-children-prop/valid-042.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-043.jsx` | `tests/lib/rules/no-children-prop.js valid[43]` | derived (`fixtures/no-children-prop/valid-043.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-children-prop/valid-044.jsx` | `tests/lib/rules/no-children-prop.js valid[44]` | derived (`fixtures/no-children-prop/valid-044.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/invalid-000.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[0]` | derived (`fixtures/no-danger-with-children/invalid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/invalid-001.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[1]` | derived (`fixtures/no-danger-with-children/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/invalid-002.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[2]` | derived (`fixtures/no-danger-with-children/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/invalid-003.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[3]` | derived (`fixtures/no-danger-with-children/invalid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/invalid-004.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[4]` | derived (`fixtures/no-danger-with-children/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/invalid-005.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[5]` | derived (`fixtures/no-danger-with-children/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/invalid-006.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[6]` | derived (`fixtures/no-danger-with-children/invalid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/invalid-007.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[7]` | derived (`fixtures/no-danger-with-children/invalid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/invalid-008.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[8]` | derived (`fixtures/no-danger-with-children/invalid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/invalid-009.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[9]` | derived (`fixtures/no-danger-with-children/invalid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/invalid-010.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[10]` | derived (`fixtures/no-danger-with-children/invalid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/invalid-011.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[11]` | derived (`fixtures/no-danger-with-children/invalid-011.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/invalid-012.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[12]` | derived (`fixtures/no-danger-with-children/invalid-012.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/invalid-013.jsx` | `tests/lib/rules/no-danger-with-children.js invalid[13]` | derived (`fixtures/no-danger-with-children/invalid-013.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/valid-002.jsx` | `tests/lib/rules/no-danger-with-children.js valid[2]` | derived (`fixtures/no-danger-with-children/valid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/valid-003.jsx` | `tests/lib/rules/no-danger-with-children.js valid[3]` | derived (`fixtures/no-danger-with-children/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/valid-004.jsx` | `tests/lib/rules/no-danger-with-children.js valid[4]` | derived (`fixtures/no-danger-with-children/valid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/valid-005.jsx` | `tests/lib/rules/no-danger-with-children.js valid[5]` | derived (`fixtures/no-danger-with-children/valid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/valid-006.jsx` | `tests/lib/rules/no-danger-with-children.js valid[6]` | derived (`fixtures/no-danger-with-children/valid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/valid-007.jsx` | `tests/lib/rules/no-danger-with-children.js valid[7]` | derived (`fixtures/no-danger-with-children/valid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/valid-008.jsx` | `tests/lib/rules/no-danger-with-children.js valid[8]` | derived (`fixtures/no-danger-with-children/valid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/valid-009.jsx` | `tests/lib/rules/no-danger-with-children.js valid[9]` | derived (`fixtures/no-danger-with-children/valid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/valid-010.jsx` | `tests/lib/rules/no-danger-with-children.js valid[10]` | derived (`fixtures/no-danger-with-children/valid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/valid-011.jsx` | `tests/lib/rules/no-danger-with-children.js valid[11]` | derived (`fixtures/no-danger-with-children/valid-011.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/valid-012.jsx` | `tests/lib/rules/no-danger-with-children.js valid[12]` | derived (`fixtures/no-danger-with-children/valid-012.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/valid-013.jsx` | `tests/lib/rules/no-danger-with-children.js valid[13]` | derived (`fixtures/no-danger-with-children/valid-013.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/valid-014.jsx` | `tests/lib/rules/no-danger-with-children.js valid[14]` | derived (`fixtures/no-danger-with-children/valid-014.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/valid-015.jsx` | `tests/lib/rules/no-danger-with-children.js valid[15]` | derived (`fixtures/no-danger-with-children/valid-015.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger-with-children/valid-016.jsx` | `tests/lib/rules/no-danger-with-children.js valid[16]` | derived (`fixtures/no-danger-with-children/valid-016.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger/invalid-000.jsx` | `tests/lib/rules/no-danger.js invalid[0]` | derived (`fixtures/no-danger/invalid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger/invalid-001.jsx` | `tests/lib/rules/no-danger.js invalid[1]` | derived (`fixtures/no-danger/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger/invalid-002.jsx` | `tests/lib/rules/no-danger.js invalid[2]` | derived (`fixtures/no-danger/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger/invalid-003.jsx` | `tests/lib/rules/no-danger.js invalid[3]` | derived (`fixtures/no-danger/invalid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger/invalid-004.jsx` | `tests/lib/rules/no-danger.js invalid[4]` | derived (`fixtures/no-danger/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger/invalid-005.jsx` | `tests/lib/rules/no-danger.js invalid[5]` | derived (`fixtures/no-danger/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger/invalid-006.tsx` | `tests/lib/rules/no-danger.js invalid[6]` | derived (`fixtures/no-danger/invalid-006.tsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger/valid-000.jsx` | `tests/lib/rules/no-danger.js valid[0]` | derived (`fixtures/no-danger/valid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger/valid-001.jsx` | `tests/lib/rules/no-danger.js valid[1]` | derived (`fixtures/no-danger/valid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger/valid-002.jsx` | `tests/lib/rules/no-danger.js valid[2]` | derived (`fixtures/no-danger/valid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger/valid-003.jsx` | `tests/lib/rules/no-danger.js valid[3]` | derived (`fixtures/no-danger/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger/valid-004.jsx` | `tests/lib/rules/no-danger.js valid[4]` | derived (`fixtures/no-danger/valid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-danger/valid-005.jsx` | `tests/lib/rules/no-danger.js valid[5]` | derived (`fixtures/no-danger/valid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/invalid-000.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[0]` | derived (`fixtures/no-direct-mutation-state/invalid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/invalid-001.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[1]` | derived (`fixtures/no-direct-mutation-state/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/invalid-002.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[2]` | derived (`fixtures/no-direct-mutation-state/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/invalid-003.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[3]` | derived (`fixtures/no-direct-mutation-state/invalid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/invalid-004.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[4]` | derived (`fixtures/no-direct-mutation-state/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/invalid-005.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[5]` | derived (`fixtures/no-direct-mutation-state/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/invalid-006.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[6]` | derived (`fixtures/no-direct-mutation-state/invalid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/invalid-007.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[7]` | derived (`fixtures/no-direct-mutation-state/invalid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/invalid-008.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[8]` | derived (`fixtures/no-direct-mutation-state/invalid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/invalid-009.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[9]` | derived (`fixtures/no-direct-mutation-state/invalid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/invalid-010.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[10]` | derived (`fixtures/no-direct-mutation-state/invalid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/invalid-011.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[11]` | derived (`fixtures/no-direct-mutation-state/invalid-011.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/invalid-012.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[12]` | derived (`fixtures/no-direct-mutation-state/invalid-012.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/invalid-013.jsx` | `tests/lib/rules/no-direct-mutation-state.js invalid[13]` | derived (`fixtures/no-direct-mutation-state/invalid-013.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/valid-000.jsx` | `tests/lib/rules/no-direct-mutation-state.js valid[0]` | derived (`fixtures/no-direct-mutation-state/valid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/valid-001.jsx` | `tests/lib/rules/no-direct-mutation-state.js valid[1]` | derived (`fixtures/no-direct-mutation-state/valid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/valid-002.jsx` | `tests/lib/rules/no-direct-mutation-state.js valid[2]` | derived (`fixtures/no-direct-mutation-state/valid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/valid-003.jsx` | `tests/lib/rules/no-direct-mutation-state.js valid[3]` | derived (`fixtures/no-direct-mutation-state/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/valid-004.jsx` | `tests/lib/rules/no-direct-mutation-state.js valid[4]` | derived (`fixtures/no-direct-mutation-state/valid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/valid-005.jsx` | `tests/lib/rules/no-direct-mutation-state.js valid[5]` | derived (`fixtures/no-direct-mutation-state/valid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-direct-mutation-state/valid-006.jsx` | `tests/lib/rules/no-direct-mutation-state.js valid[6]` | derived (`fixtures/no-direct-mutation-state/valid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-find-dom-node/invalid-000.jsx` | `tests/lib/rules/no-find-dom-node.js invalid[0]` | derived (`fixtures/no-find-dom-node/invalid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-find-dom-node/invalid-001.jsx` | `tests/lib/rules/no-find-dom-node.js invalid[1]` | derived (`fixtures/no-find-dom-node/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-find-dom-node/invalid-002.jsx` | `tests/lib/rules/no-find-dom-node.js invalid[2]` | derived (`fixtures/no-find-dom-node/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-find-dom-node/invalid-003.jsx` | `tests/lib/rules/no-find-dom-node.js invalid[3]` | derived (`fixtures/no-find-dom-node/invalid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-find-dom-node/valid-000.jsx` | `tests/lib/rules/no-find-dom-node.js valid[0]` | derived (`fixtures/no-find-dom-node/valid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-find-dom-node/valid-001.jsx` | `tests/lib/rules/no-find-dom-node.js valid[1]` | derived (`fixtures/no-find-dom-node/valid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-find-dom-node/valid-002.jsx` | `tests/lib/rules/no-find-dom-node.js valid[2]` | derived (`fixtures/no-find-dom-node/valid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-find-dom-node/valid-003.jsx` | `tests/lib/rules/no-find-dom-node.js valid[3]` | derived (`fixtures/no-find-dom-node/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-is-mounted/invalid-000.jsx` | `tests/lib/rules/no-is-mounted.js invalid[0]` | derived (`fixtures/no-is-mounted/invalid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-is-mounted/invalid-001.jsx` | `tests/lib/rules/no-is-mounted.js invalid[1]` | derived (`fixtures/no-is-mounted/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-is-mounted/invalid-002.jsx` | `tests/lib/rules/no-is-mounted.js invalid[2]` | derived (`fixtures/no-is-mounted/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-is-mounted/valid-000.jsx` | `tests/lib/rules/no-is-mounted.js valid[0]` | derived (`fixtures/no-is-mounted/valid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-is-mounted/valid-001.jsx` | `tests/lib/rules/no-is-mounted.js valid[1]` | derived (`fixtures/no-is-mounted/valid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-is-mounted/valid-002.jsx` | `tests/lib/rules/no-is-mounted.js valid[2]` | derived (`fixtures/no-is-mounted/valid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-is-mounted/valid-003.jsx` | `tests/lib/rules/no-is-mounted.js valid[3]` | derived (`fixtures/no-is-mounted/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/invalid-000.jsx` | `tests/lib/rules/no-render-return-value.js invalid[0]` | derived (`fixtures/no-render-return-value/invalid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/invalid-001.jsx` | `tests/lib/rules/no-render-return-value.js invalid[1]` | derived (`fixtures/no-render-return-value/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/invalid-002.jsx` | `tests/lib/rules/no-render-return-value.js invalid[2]` | derived (`fixtures/no-render-return-value/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/invalid-003.jsx` | `tests/lib/rules/no-render-return-value.js invalid[3]` | derived (`fixtures/no-render-return-value/invalid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/invalid-004.jsx` | `tests/lib/rules/no-render-return-value.js invalid[4]` | derived (`fixtures/no-render-return-value/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/invalid-005.jsx` | `tests/lib/rules/no-render-return-value.js invalid[5]` | derived (`fixtures/no-render-return-value/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/invalid-006.jsx` | `tests/lib/rules/no-render-return-value.js invalid[6]` | derived (`fixtures/no-render-return-value/invalid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/invalid-007.jsx` | `tests/lib/rules/no-render-return-value.js invalid[7]` | derived (`fixtures/no-render-return-value/invalid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/invalid-008.jsx` | `tests/lib/rules/no-render-return-value.js invalid[8]` | derived (`fixtures/no-render-return-value/invalid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/valid-000.jsx` | `tests/lib/rules/no-render-return-value.js valid[0]` | derived (`fixtures/no-render-return-value/valid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/valid-001.jsx` | `tests/lib/rules/no-render-return-value.js valid[1]` | derived (`fixtures/no-render-return-value/valid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/valid-002.jsx` | `tests/lib/rules/no-render-return-value.js valid[2]` | derived (`fixtures/no-render-return-value/valid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/valid-003.jsx` | `tests/lib/rules/no-render-return-value.js valid[3]` | derived (`fixtures/no-render-return-value/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/valid-004.jsx` | `tests/lib/rules/no-render-return-value.js valid[4]` | derived (`fixtures/no-render-return-value/valid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/valid-005.jsx` | `tests/lib/rules/no-render-return-value.js valid[5]` | derived (`fixtures/no-render-return-value/valid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/valid-006.jsx` | `tests/lib/rules/no-render-return-value.js valid[6]` | derived (`fixtures/no-render-return-value/valid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-render-return-value/valid-007.jsx` | `tests/lib/rules/no-render-return-value.js valid[7]` | derived (`fixtures/no-render-return-value/valid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-string-refs/invalid-000.jsx` | `tests/lib/rules/no-string-refs.js invalid[0]` | derived (`fixtures/no-string-refs/invalid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-string-refs/invalid-001.jsx` | `tests/lib/rules/no-string-refs.js invalid[1]` | derived (`fixtures/no-string-refs/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-string-refs/invalid-002.jsx` | `tests/lib/rules/no-string-refs.js invalid[2]` | derived (`fixtures/no-string-refs/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-string-refs/invalid-003.jsx` | `tests/lib/rules/no-string-refs.js invalid[3]` | derived (`fixtures/no-string-refs/invalid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-string-refs/invalid-004.jsx` | `tests/lib/rules/no-string-refs.js invalid[4]` | derived (`fixtures/no-string-refs/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-string-refs/invalid-005.jsx` | `tests/lib/rules/no-string-refs.js invalid[5]` | derived (`fixtures/no-string-refs/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-string-refs/invalid-006.jsx` | `tests/lib/rules/no-string-refs.js invalid[6]` | derived (`fixtures/no-string-refs/invalid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-string-refs/valid-000.jsx` | `tests/lib/rules/no-string-refs.js valid[0]` | derived (`fixtures/no-string-refs/valid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-string-refs/valid-001.jsx` | `tests/lib/rules/no-string-refs.js valid[1]` | derived (`fixtures/no-string-refs/valid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-string-refs/valid-002.jsx` | `tests/lib/rules/no-string-refs.js valid[2]` | derived (`fixtures/no-string-refs/valid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-string-refs/valid-003.jsx` | `tests/lib/rules/no-string-refs.js valid[3]` | derived (`fixtures/no-string-refs/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-unescaped-entities/invalid-002.jsx` | `tests/lib/rules/no-unescaped-entities.js invalid[2]` | derived (`fixtures/no-unescaped-entities/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-unescaped-entities/invalid-004.jsx` | `tests/lib/rules/no-unescaped-entities.js invalid[4]` | derived (`fixtures/no-unescaped-entities/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-unescaped-entities/invalid-005.jsx` | `tests/lib/rules/no-unescaped-entities.js invalid[5]` | derived (`fixtures/no-unescaped-entities/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-unescaped-entities/invalid-006.jsx` | `tests/lib/rules/no-unescaped-entities.js invalid[6]` | derived (`fixtures/no-unescaped-entities/invalid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-unescaped-entities/valid-000.jsx` | `tests/lib/rules/no-unescaped-entities.js valid[0]` | derived (`fixtures/no-unescaped-entities/valid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-unescaped-entities/valid-001.jsx` | `tests/lib/rules/no-unescaped-entities.js valid[1]` | derived (`fixtures/no-unescaped-entities/valid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-unescaped-entities/valid-002.jsx` | `tests/lib/rules/no-unescaped-entities.js valid[2]` | derived (`fixtures/no-unescaped-entities/valid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-unescaped-entities/valid-003.jsx` | `tests/lib/rules/no-unescaped-entities.js valid[3]` | derived (`fixtures/no-unescaped-entities/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-unescaped-entities/valid-004.jsx` | `tests/lib/rules/no-unescaped-entities.js valid[4]` | derived (`fixtures/no-unescaped-entities/valid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-unescaped-entities/valid-005.jsx` | `tests/lib/rules/no-unescaped-entities.js valid[5]` | derived (`fixtures/no-unescaped-entities/valid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-unescaped-entities/valid-006.jsx` | `tests/lib/rules/no-unescaped-entities.js valid[6]` | derived (`fixtures/no-unescaped-entities/valid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/no-unescaped-entities/valid-007.jsx` | `tests/lib/rules/no-unescaped-entities.js valid[7]` | derived (`fixtures/no-unescaped-entities/valid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/react-in-jsx-scope/invalid-000.jsx` | `tests/lib/rules/react-in-jsx-scope.js invalid[0]` | derived (`fixtures/react-in-jsx-scope/invalid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/react-in-jsx-scope/invalid-001.jsx` | `tests/lib/rules/react-in-jsx-scope.js invalid[1]` | derived (`fixtures/react-in-jsx-scope/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/react-in-jsx-scope/invalid-002.jsx` | `tests/lib/rules/react-in-jsx-scope.js invalid[2]` | derived (`fixtures/react-in-jsx-scope/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/react-in-jsx-scope/invalid-003.jsx` | `tests/lib/rules/react-in-jsx-scope.js invalid[3]` | derived (`fixtures/react-in-jsx-scope/invalid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/react-in-jsx-scope/invalid-004.jsx` | `tests/lib/rules/react-in-jsx-scope.js invalid[4]` | derived (`fixtures/react-in-jsx-scope/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/react-in-jsx-scope/invalid-005.jsx` | `tests/lib/rules/react-in-jsx-scope.js invalid[5]` | derived (`fixtures/react-in-jsx-scope/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/react-in-jsx-scope/invalid-006.jsx` | `tests/lib/rules/react-in-jsx-scope.js invalid[6]` | derived (`fixtures/react-in-jsx-scope/invalid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/react-in-jsx-scope/valid-000.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[0]` | derived (`fixtures/react-in-jsx-scope/valid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/react-in-jsx-scope/valid-004.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[4]` | derived (`fixtures/react-in-jsx-scope/valid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/react-in-jsx-scope/valid-005.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[5]` | derived (`fixtures/react-in-jsx-scope/valid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/react-in-jsx-scope/valid-006.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[6]` | derived (`fixtures/react-in-jsx-scope/valid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/react-in-jsx-scope/valid-007.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[7]` | derived (`fixtures/react-in-jsx-scope/valid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/react-in-jsx-scope/valid-008.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[8]` | derived (`fixtures/react-in-jsx-scope/valid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/react-in-jsx-scope/valid-009.jsx` | `tests/lib/rules/react-in-jsx-scope.js valid[9]` | derived (`fixtures/react-in-jsx-scope/valid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/invalid-000.jsx` | `tests/lib/rules/self-closing-comp.js invalid[0]` | derived (`fixtures/self-closing-comp/invalid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/invalid-001.jsx` | `tests/lib/rules/self-closing-comp.js invalid[1]` | derived (`fixtures/self-closing-comp/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/invalid-002.jsx` | `tests/lib/rules/self-closing-comp.js invalid[2]` | derived (`fixtures/self-closing-comp/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/invalid-003.jsx` | `tests/lib/rules/self-closing-comp.js invalid[3]` | derived (`fixtures/self-closing-comp/invalid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/invalid-004.jsx` | `tests/lib/rules/self-closing-comp.js invalid[4]` | derived (`fixtures/self-closing-comp/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/invalid-005.jsx` | `tests/lib/rules/self-closing-comp.js invalid[5]` | derived (`fixtures/self-closing-comp/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/invalid-006.jsx` | `tests/lib/rules/self-closing-comp.js invalid[6]` | derived (`fixtures/self-closing-comp/invalid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/invalid-007.jsx` | `tests/lib/rules/self-closing-comp.js invalid[7]` | derived (`fixtures/self-closing-comp/invalid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/invalid-008.jsx` | `tests/lib/rules/self-closing-comp.js invalid[8]` | derived (`fixtures/self-closing-comp/invalid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/invalid-009.jsx` | `tests/lib/rules/self-closing-comp.js invalid[9]` | derived (`fixtures/self-closing-comp/invalid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/invalid-010.jsx` | `tests/lib/rules/self-closing-comp.js invalid[10]` | derived (`fixtures/self-closing-comp/invalid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/invalid-011.jsx` | `tests/lib/rules/self-closing-comp.js invalid[11]` | derived (`fixtures/self-closing-comp/invalid-011.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-000.jsx` | `tests/lib/rules/self-closing-comp.js valid[0]` | derived (`fixtures/self-closing-comp/valid-000.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-001.jsx` | `tests/lib/rules/self-closing-comp.js valid[1]` | derived (`fixtures/self-closing-comp/valid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-002.jsx` | `tests/lib/rules/self-closing-comp.js valid[2]` | derived (`fixtures/self-closing-comp/valid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-003.jsx` | `tests/lib/rules/self-closing-comp.js valid[3]` | derived (`fixtures/self-closing-comp/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-004.jsx` | `tests/lib/rules/self-closing-comp.js valid[4]` | derived (`fixtures/self-closing-comp/valid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-005.jsx` | `tests/lib/rules/self-closing-comp.js valid[5]` | derived (`fixtures/self-closing-comp/valid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-006.jsx` | `tests/lib/rules/self-closing-comp.js valid[6]` | derived (`fixtures/self-closing-comp/valid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-007.jsx` | `tests/lib/rules/self-closing-comp.js valid[7]` | derived (`fixtures/self-closing-comp/valid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-008.jsx` | `tests/lib/rules/self-closing-comp.js valid[8]` | derived (`fixtures/self-closing-comp/valid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-009.jsx` | `tests/lib/rules/self-closing-comp.js valid[9]` | derived (`fixtures/self-closing-comp/valid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-010.jsx` | `tests/lib/rules/self-closing-comp.js valid[10]` | derived (`fixtures/self-closing-comp/valid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-011.jsx` | `tests/lib/rules/self-closing-comp.js valid[11]` | derived (`fixtures/self-closing-comp/valid-011.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-012.jsx` | `tests/lib/rules/self-closing-comp.js valid[12]` | derived (`fixtures/self-closing-comp/valid-012.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-013.jsx` | `tests/lib/rules/self-closing-comp.js valid[13]` | derived (`fixtures/self-closing-comp/valid-013.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-014.jsx` | `tests/lib/rules/self-closing-comp.js valid[14]` | derived (`fixtures/self-closing-comp/valid-014.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-015.jsx` | `tests/lib/rules/self-closing-comp.js valid[15]` | derived (`fixtures/self-closing-comp/valid-015.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-016.jsx` | `tests/lib/rules/self-closing-comp.js valid[16]` | derived (`fixtures/self-closing-comp/valid-016.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-017.jsx` | `tests/lib/rules/self-closing-comp.js valid[17]` | derived (`fixtures/self-closing-comp/valid-017.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-018.jsx` | `tests/lib/rules/self-closing-comp.js valid[18]` | derived (`fixtures/self-closing-comp/valid-018.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-019.jsx` | `tests/lib/rules/self-closing-comp.js valid[19]` | derived (`fixtures/self-closing-comp/valid-019.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-020.jsx` | `tests/lib/rules/self-closing-comp.js valid[20]` | derived (`fixtures/self-closing-comp/valid-020.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-021.jsx` | `tests/lib/rules/self-closing-comp.js valid[21]` | derived (`fixtures/self-closing-comp/valid-021.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-022.jsx` | `tests/lib/rules/self-closing-comp.js valid[22]` | derived (`fixtures/self-closing-comp/valid-022.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-023.jsx` | `tests/lib/rules/self-closing-comp.js valid[23]` | derived (`fixtures/self-closing-comp/valid-023.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-024.jsx` | `tests/lib/rules/self-closing-comp.js valid[24]` | derived (`fixtures/self-closing-comp/valid-024.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-025.jsx` | `tests/lib/rules/self-closing-comp.js valid[25]` | derived (`fixtures/self-closing-comp/valid-025.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-026.jsx` | `tests/lib/rules/self-closing-comp.js valid[26]` | derived (`fixtures/self-closing-comp/valid-026.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-027.jsx` | `tests/lib/rules/self-closing-comp.js valid[27]` | derived (`fixtures/self-closing-comp/valid-027.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-028.jsx` | `tests/lib/rules/self-closing-comp.js valid[28]` | derived (`fixtures/self-closing-comp/valid-028.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-029.jsx` | `tests/lib/rules/self-closing-comp.js valid[29]` | derived (`fixtures/self-closing-comp/valid-029.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-030.jsx` | `tests/lib/rules/self-closing-comp.js valid[30]` | derived (`fixtures/self-closing-comp/valid-030.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-031.jsx` | `tests/lib/rules/self-closing-comp.js valid[31]` | derived (`fixtures/self-closing-comp/valid-031.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-032.jsx` | `tests/lib/rules/self-closing-comp.js valid[32]` | derived (`fixtures/self-closing-comp/valid-032.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-033.jsx` | `tests/lib/rules/self-closing-comp.js valid[33]` | derived (`fixtures/self-closing-comp/valid-033.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/self-closing-comp/valid-034.jsx` | `tests/lib/rules/self-closing-comp.js valid[34]` | derived (`fixtures/self-closing-comp/valid-034.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/invalid-001.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[1]` | derived (`fixtures/void-dom-elements-no-children/invalid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/invalid-002.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[2]` | derived (`fixtures/void-dom-elements-no-children/invalid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/invalid-003.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[3]` | derived (`fixtures/void-dom-elements-no-children/invalid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/invalid-004.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[4]` | derived (`fixtures/void-dom-elements-no-children/invalid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/invalid-005.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[5]` | derived (`fixtures/void-dom-elements-no-children/invalid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/invalid-006.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[6]` | derived (`fixtures/void-dom-elements-no-children/invalid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/invalid-007.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[7]` | derived (`fixtures/void-dom-elements-no-children/invalid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/invalid-008.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[8]` | derived (`fixtures/void-dom-elements-no-children/invalid-008.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/invalid-009.jsx` | `tests/lib/rules/void-dom-elements-no-children.js invalid[9]` | derived (`fixtures/void-dom-elements-no-children/invalid-009.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/valid-001.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[1]` | derived (`fixtures/void-dom-elements-no-children/valid-001.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/valid-002.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[2]` | derived (`fixtures/void-dom-elements-no-children/valid-002.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/valid-003.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[3]` | derived (`fixtures/void-dom-elements-no-children/valid-003.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/valid-004.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[4]` | derived (`fixtures/void-dom-elements-no-children/valid-004.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/valid-005.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[5]` | derived (`fixtures/void-dom-elements-no-children/valid-005.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/valid-006.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[6]` | derived (`fixtures/void-dom-elements-no-children/valid-006.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/valid-007.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[7]` | derived (`fixtures/void-dom-elements-no-children/valid-007.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/valid-010.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[10]` | derived (`fixtures/void-dom-elements-no-children/valid-010.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/valid-011.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[11]` | derived (`fixtures/void-dom-elements-no-children/valid-011.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/valid-012.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[12]` | derived (`fixtures/void-dom-elements-no-children/valid-012.jsx` with its names and literals renamed) |
| `upstream/eslint-plugin-react/fixtures/renamed/void-dom-elements-no-children/valid-013.jsx` | `tests/lib/rules/void-dom-elements-no-children.js valid[13]` | derived (`fixtures/void-dom-elements-no-children/valid-013.jsx` with its names and literals renamed) |
