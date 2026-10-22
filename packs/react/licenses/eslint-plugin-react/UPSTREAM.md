# eslint-plugin-react 7.37.5

Upstream: [jsx-eslint/eslint-plugin-react](https://github.com/jsx-eslint/eslint-plugin-react/tree/2c98b83c451a4297edf1787d9a616e50687e27e8) at tag `v7.37.5` (commit `2c98b83c451a4297edf1787d9a616e50687e27e8`). License: MIT (Copyright (c) 2014 Yannick Croissant); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at that commit.

The files listed below are the upstream tests of the rules the React pack records (`conformance.json`, `upstream/common.mjs` `RULES`), and the test cases extracted from them, one case per file, and remain under eslint-plugin-react's license. Every other file under `upstream/eslint-plugin-react/` is Rerail's, under the Apache License 2.0 (the pack's `LICENSE`): `generate.mjs` (with its stub RuleTester), `../common.mjs` and the recordings they write by running the pinned plugin. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of eslint-plugin-react's and eslint-plugin-react-hooks' rules, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them.

Paths are relative to the pack directory, `packs/react/`. `<file> valid[i]` / `invalid[i]` is the i-th case of the `valid` / `invalid` list the test file passes to `RuleTester.run`. A `derived` row is a case's `code` that the port readiness gates' recorder (`npm run port-readiness:record`) copied from that upstream test file (at upstream's repository path), as upstream's own test helpers build the case: `tests/helpers/parsers.js` copies each case once per parser and appends a `// features: [...], parser: ...` comment to its code.

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
| `upstream/eslint-plugin-react/fixtures/button-has-type/02539b10df74.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/02c5e41e7bcc.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/04a927813af4.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/06ee86519f50.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/070716aac191.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/076a2be0b3ce.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/087c1d6211dd.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/089024b05e51.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/094eb0b35f3a.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/09a7e29a44fe.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/0b754d66d451.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/0f5b949817f5.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/0f75db076fec.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/0fc1281f6064.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/144ae0eb7ce8.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/153b13e8ecc1.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/1576325b1918.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/15bdf5197ded.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/1629ca48d21d.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/1945be6a497b.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/1b2c156bea46.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/1d825e039e4a.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/1dd88f94c339.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/1f7f348a18db.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/213395e56a23.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/21d34f358c82.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/231fb91e4357.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/2505efaadad2.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/26462c103b0d.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/26f62297d90a.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/279220f10352.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/2a5b3be369fe.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/2a74cdfb6eba.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/2be6b03c6657.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/2c0ee81cf5bc.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/2d4827c23b7d.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/310fff6e9ddf.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/340f8bbf43ea.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/34eec3ee3b3f.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/3557c8bf69bb.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/35a95a8edd90.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/36f7cf6e6c86.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/397f2d4018a3.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/39bc17c87fe0.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/3a08579ada72.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/3bdac3544660.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/3cd345ec4bb2.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/40cb81ed2be3.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/417a677f6b41.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/41dcee95da37.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/42fccf218ab2.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/4311940116ce.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/453af723c06c.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/4829a1524f1b.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/4bae3dfb66dd.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/4becda2ab9a7.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/4d254925ca98.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/4ef8ba7b0db0.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/51a702c4e943.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/53ee4a1eab8d.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/55b151633c7f.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/597eee673fdf.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/5a1311633e4b.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/5a154acf52f7.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/5ba69b0449bf.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/5c451e0389c6.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/5c89a4c9a25a.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/5f282b5631fd.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/62c92cf2f4be.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/644f3353cfcb.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/6554eb6be73d.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/6587751378dc.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/667a5c82f605.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/6a69eb273b43.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/6b2d9c97ed16.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/6da66d4b0345.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/6e655a0b1f66.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/6f0b3ab57b4e.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/730268e31032.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/7390df9dfac8.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/74451715db7e.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/76df549a1864.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/7792d49220e1.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/7921dee606f3.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/7954b1766cf5.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/7a94177f5b05.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/7baca72bab0f.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/8032fbdeffb5.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/80984e5720d3.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/821726d572ab.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/8308b1e56e24.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/83b2d689a216.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/83bdcf99d658.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/85aeac58d806.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/85bc1f6f4cc7.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/8e4b4d874bcd.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/8e757e0767b2.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/8ffc6eb3974d.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/91e7a410f145.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/91ef394177b9.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/93d439054d4a.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/941dca6a4a7c.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/960cc014053d.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/9c475cb7af6f.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/9c491814c2d0.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/9cf60e6b9427.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/9e4c0f332586.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/a54cb84da816.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/a64e2d4d8976.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/a6d4257f461a.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/a958a66a0fec.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/a98e99b99c7d.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/ab5e856359ab.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/ac7a792aa408.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/b1b6bbcb06a9.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/b208cd1cece8.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/b28584c13e91.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/b586685ee258.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/b6be4f261fa1.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/b74c47a84ba5.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/b8cde70392d9.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/ba695bf6f448.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/bb4007e82b9b.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/bca9624c4b0a.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/bd2dbba9c6df.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/bd36fbb14798.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/bda77f453e41.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/c3035bfbb674.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/c314c23e9f76.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/c4050e327e3f.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/c7cb43b12fcb.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/c90fc90a4f02.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/ca6230dfde1f.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/cc58d6bdb89e.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/cc96ea4e3e4e.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/cd63b9f3ab01.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/cd981fe90d83.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/ce4693046a33.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/cf583f10e6ed.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/d093c54dbc32.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/d113f8a82a42.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/d1e38a4a6317.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/d34e0089f406.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/d60e6973f051.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/d7b11ed9b78a.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/dab02eedda93.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/dba8922fccd8.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/dbee508446c5.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/dc01b395197c.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/dde13874fe95.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/e04017978aa6.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/e256d28527ae.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/e29931610cb2.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/e4311ed952b6.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/e615f14116ee.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/e83337c90011.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/eab5aeac0470.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/eae4b174ee78.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/ec443bc736a5.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/ecf49e61aad2.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/efe54fa1643b.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/f0dcf27e58aa.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/f1c7e38f9d72.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/f438d622b007.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/f64d9704d418.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/f6665fc83adc.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/f6bb004b2c0d.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/f81f9f68aa44.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/fa0895a65228.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/fc5221d5360c.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/fcf0d3521a74.jsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/ff27fb2bde63.tsx` | `tests/lib/rules/button-has-type.js` | derived |
| `upstream/eslint-plugin-react/fixtures/button-has-type/ff783040737a.jsx` | `tests/lib/rules/button-has-type.js` | derived |
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
| `upstream/eslint-plugin-react/fixtures/jsx-key/001885a27142.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/006ac764a19e.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/00bb8e008d80.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/01aa478304e4.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/02e8b8b45c49.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/045c35616f90.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/05e4283a3b45.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/0630080ab414.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/086a4d949e8f.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/09de134daa13.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/0abc7cfb090a.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/0bcd8f933fd4.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/0c2a32560353.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/0d00c7e2e130.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/0e330947276b.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/0fb083e3da8b.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/1284abec73a3.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/137ecb157fc6.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/13c02af8d3f5.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/14345c87cea2.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/1554ca808161.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/1582bd92ad81.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/1584cd802ee5.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/199b06649a16.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/1b143b57e236.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/1bf2c4ab2adf.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/1d26ed0957d8.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/1fa607f336d2.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/2105568c82e7.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/21b5951ed765.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/222d7c612904.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/2281478031f5.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/23d52e45a453.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/23ebab0ed5ad.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/24c274ead152.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/262bdaed6854.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/27c84db9e55b.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/2843d68cf9be.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/28d4fff9767a.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/29c827a4b91f.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/2bc0a1c3d19b.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/2dd1b42aba42.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/337f0fe9d3ac.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/345f22ec90bd.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/35a833131e3d.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/3844542b70b2.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/3970b1451b7c.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/3a07dacbb9af.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/3b0efd81d3d1.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/3bf2be9c3770.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/3f57bb1cdd83.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/401e94bd2480.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/41a0faadbba5.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/433a0aed6883.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/439bd4c57546.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/477998bec1f9.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/47af537cdd21.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/4bf58144591c.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/4c01ec963cae.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/4faf241835ab.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/514a934b5c1e.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/528d2b3222fc.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/53544df64b56.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/53eff020d8ed.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/543e5265e712.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/54566370512c.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/5493e49fb8f9.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/599b6ef98813.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/59e92ce1c5ed.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/5a05b788eb56.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/5b674524c7e8.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/5b8a2a5c85f6.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/5ed158ad917b.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/5f4abd086648.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/636875866d4e.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/64527084128c.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/6b8d32ac084d.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/6f49b89fa8ef.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/71fe23a12445.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/7214682a4400.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/72cda7cf1add.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/743390ac2a4a.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/75d07a882302.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/782a80b1d98a.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/790dfbf90c24.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/7d55a680b513.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/7f27ea46fab3.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/80ce9841b046.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/8210c4da23a7.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/833cb09e3f0c.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/848a7dffd179.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/85fcbef0f94e.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/86c32c5409d2.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/879aab8a38d1.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/8861a199daeb.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/89cda5bf03a1.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/8a181c42b33b.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/8b2f5e9cd846.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/8c0c8db80c02.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/8e6d6dfac4d4.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/8fd7ae385055.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/9018effc5419.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/92f06145174f.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/9310dfab54a6.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/937d5a7e9ce5.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/949713d5f411.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/94f557330899.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/972fb6520a2c.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/99912d75b4ff.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/99a661ef3952.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/9a4eb0e1a17f.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/9d4fe4ddf997.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/9ee49a31a881.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/9f3a9c45589f.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/9fa00e7d718c.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/a1d283376d98.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/a25243a6514d.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/a2bed38290cf.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/a350a06df3b6.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/a472bfff66a0.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/a882fa9526f1.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/aaaa2aab82de.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/aac63c4be502.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/af4fc3c19fea.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/b1c1d1de274e.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/b35c8675e053.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/b7d31800fad1.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/b9232bf53d42.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/bea19d05e474.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/bf2b88c189a4.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/c026686cda78.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/c1df673907a5.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/c24e651584ca.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/c3a08a67761c.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/c59ef1e8434a.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/c662996203f3.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/c8d3efb0ed70.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/c96551e9cc34.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/ca6835eab1fd.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/cd78486cf5f4.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/cd828b298e73.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/cf6d6904ffe0.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/d45b2fdfebc4.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/d88bce05c3bd.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/dac093a88bc1.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/dcde2235564f.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/ddbf13a236fb.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/e1e952ee383c.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/e28b26c6c7fb.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/e5f579db2da5.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/e816c826d434.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/e86180f2b8ae.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/e9dbadee81a3.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/ebb67c519ca7.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/ec83ac0ebb0b.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/ee58a070cc6c.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/f2277068fa89.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/f2b1e5d87be9.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/f77257e08fb5.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/f84282956362.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/f87bd8ab7bd7.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/f951601da1f4.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/f9fda1459e1c.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/fafad630e3a9.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/fb6d12c7765e.jsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/fc2ca242fe12.tsx` | `tests/lib/rules/jsx-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-key/fe51e8c10e7c.tsx` | `tests/lib/rules/jsx-key.js` | derived |
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
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/0030273697a4.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/04922eb74334.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/111316655011.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/1476e8fbc993.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/185eeb5e4c89.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/191968deb047.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/194de17ddb7b.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/1aceed5c85bb.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/1d8dc6b31381.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/1e5f8aef4178.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/1f9f68185f7b.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/25a9cf022eb0.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/28a4f04bd603.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/2cb8194dc390.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/2de28567b346.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/3489cfe261c3.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/3676c13e2297.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/38c3a312e855.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/3f9868caab7c.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/4123c9ea01a1.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/4976b0177685.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/4a513714378d.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/4d9329d23821.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/4f5df6edb6b4.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/506e7c74cb41.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/53261addad45.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/565ac443dcf4.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/5b872d417e84.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/5f78a16db0d7.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/607151f42aa8.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/6935a80b5c8a.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/6ef9057bcb8f.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/70c113ab1b0b.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/714898d51d9a.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/73b0a144122b.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/769873dacfb9.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/7b2acfe15648.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/7c6af090b9ff.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/7f123f63b9d8.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/7fc97909dee6.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/88f55eee350e.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/8a9afcaa8bda.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/8e51fe5fe4f6.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/96655ad83bae.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/99c27c83fb66.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/9ddfd73dd4ec.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/9e976c346dd7.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/a08a9a011cd4.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/a173f5d84e05.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/ae25fab8d1a4.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/ae638aaa8791.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/bd36377dc162.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/bd54854084c3.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/beba612a0c60.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/c053f88bcaca.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/c5ebc30f7990.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/cb5f0f532f05.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/cbe81849077c.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/da8a7bc090d8.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/de05709d8c4a.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/de4de8372203.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/dff398672960.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/e0753c1b092e.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/f6aa4df51291.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/fca162d706f9.tsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-comment-textnodes/fdd7baf1af12.jsx` | `tests/lib/rules/jsx-no-comment-textnodes.js` | derived |
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
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/05e4283a3b45.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/126bf018f8ff.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/132eb5fa26b5.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/14072d215da3.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/14b80c8089d0.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/1aca1836f838.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/223e42313c4b.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/2708b9e2dac4.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/2ae410e5813f.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/337f5f7e1cac.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/33fcb366e615.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/367920ff8f84.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/36cfa2fb8a35.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/445e52acd204.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/4e183ed8647f.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/534149d99261.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/540d80ecf174.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/58422273d04e.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/5fdf06d528bb.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/6286ff299312.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/63b46f4e55cd.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/775ee8b7fcff.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/7b72c43ed403.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/7d871f87dba3.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/7f327b3a79e0.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/7fe0a89d5de3.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/82e6e0881040.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/8339a3834cb8.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/852eed529b10.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/8a23794c26fb.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/8a669ac71934.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/8b17cb0e7241.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/8faebd979dd3.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/976bda34f30c.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/978dc5424215.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/9a64e3559175.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/9caf98ccb455.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/a1147e2c59d0.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/a1d283376d98.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/a4987d5f29ba.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/a6001fa35ae6.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/af4fc3c19fea.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/b1320d7f114a.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/b13de28d8fed.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/c1f608320837.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/c2595c3a5148.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/cfa59571a13c.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/d008dfec9be5.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/d183aab1abc7.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/d3e3f8e7b14f.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/dcc784b0671c.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/e09e7d7cbbac.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/e2b31576bb29.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/e3183f66119d.jsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-duplicate-props/f8c376a912c3.tsx` | `tests/lib/rules/jsx-no-duplicate-props.js` | derived |
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
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/006c4d33cf3b.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/018eb17a3dcb.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/02902a694dc9.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/042ed69ecd9b.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/04c912750898.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/04efbda1ab7c.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/065e0c81aa97.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/07853289dfe7.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/089be69b5be4.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/08b84c8e40a1.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/08e7a69b70a2.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/0965b5e935c2.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/09b945140718.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/09d12c3cb75d.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/0a8347d2becd.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/0b039a1be8e4.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/0b930e30ebcd.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/0cd7516301b6.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/0e1bc7376e88.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/0e59ba5575b8.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/0e66e199718c.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/0eb11a0d2f67.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/0f6be22243df.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/104737af5fc2.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/107ea0c66d1a.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/112772324ee0.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/118a0ab0fbe0.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/118a27af8aee.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/119f32634c3e.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/11c7ef3c08d7.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/12006e758927.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/13c086f7f8bc.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/154d6117416c.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/17754fbd2468.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/18b2b3007f4c.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/19dc84b75115.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/1c5c7d77df12.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/1d0eaaa04822.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/1d7c4cd5f815.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/1f2117d8a80e.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/1fcf2d7c0058.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/1fe592687cbe.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/211188c31486.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/2175705f77fb.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/2198a3d50068.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/2530bd5e8600.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/257708c9750c.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/271e2b1082a4.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/2764f2acf334.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/2765ff97a8d1.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/28460b324e36.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/29976c72d1d2.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/2b00a1100049.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/2be18ca359b1.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/2cc162a8f83c.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/2cd64411e6f9.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/2f2d6474ced9.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/2f9b7c8983c2.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/2ff8702e2543.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/3068f86977cc.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/308ae7f8c77d.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/30add0579d37.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/30cfbbbaab40.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/30de453087b1.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/310409895aef.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/31105906aede.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/3263cf759141.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/32f17e519ca1.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/3534ad9ed62a.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/365b8d52e000.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/37dd375e1282.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/3997e75c3367.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/3a0b01978480.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/3a0cf055a697.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/3a33ff21c802.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/3a8727d66a5e.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/3b39cf0e0b76.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/3d6d6ccbeda5.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/3e09b792f7ed.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/3ec094394ee9.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/3f530eb958fd.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/3f707098f29f.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/4063ff479ed3.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/407846388777.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/42223b1826bd.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/4326e2accf20.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/43c2b45d036a.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/43db8130c243.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/449abd9e6571.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/44b40f3aab47.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/4500c60962b7.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/45dafafda4ed.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/4621c4aa7c94.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/4674f5a10f49.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/46f7c5122ff1.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/475cfaf21f1a.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/47a24c663508.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/48fec47b16a5.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/49d95bd0451b.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/49de843a4c7e.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/4a8e8f5be9c7.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/4a934d9151fe.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/4ac77e2e544a.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/4c8ec7a66e81.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/4ca41f20572f.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/4d28645caf69.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/4d9b53ecf702.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/4dbe1cea82d6.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/4ed36e322669.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/4f918ebf6861.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/5071ef38aa8f.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/51f9100eeda4.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/52347b7a9bf3.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/55601f755434.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/5635f69c3c35.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/56e7e7dd98c4.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/57e24ef43f15.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/58bdcb8967a1.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/58eb6b431864.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/5948a60a75bb.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/59698b0ad374.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/59bbc9dbc8c0.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/5a18bc97a448.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/5b82446d2a56.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/5bfd78628c2e.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/5c6af435ec5e.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/5cc293c08f74.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/5d3218146a26.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/5e6f06e9a6c5.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/5efe36630ae9.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/5f4e4a92da2f.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/60239ba254e1.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/60a4992d8130.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/62698e42a6a8.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/6308bff6ffd5.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/64508a490fc7.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/6757f8684b98.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/686137925cba.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/6920f97f7e8b.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/694a5f720be5.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/694ffadb7f47.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/6aa022622a84.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/6aaf5806d31c.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/6bace2669599.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/6c435fd03f25.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/6e147255a402.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/6ea1b20bd771.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/6ea5ff93fe7d.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/6eec2aae3d33.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/6fdcfa65e858.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/705441b8b23f.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/7078c35161d9.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/71f38129059d.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/7469029fd401.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/74796127c06e.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/74c60214ac9e.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/75b603d3d13d.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/75c0b4a2ec26.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/76a1f1cc0423.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/799817bcc7f9.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/79a45615bd55.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/7a37666f5db1.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/7a76f8b2a01f.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/7d40e34fb5a7.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/7d68a0a3d69d.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/7d806ffa97ab.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/7f9a527278cb.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/80434405d4d3.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/80d2fea76812.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/80d86f81cf9e.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/817c52a49aa3.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/81d339ada5a1.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/82378a3329c5.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/832fcfcf06b0.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/84351307134d.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/84e562d5f265.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/84ff4a71f3cb.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/859d3632db01.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/8641e0d8e15a.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/87baa3ef9495.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/8872f566c3fb.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/888acd6f74af.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/889804821418.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/88ac8813db8a.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/89170c99c56e.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/899da758c5ba.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/89f2e13fe5ed.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/8a295d219fd7.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/8b9b672c589a.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/8bf6d77f3ad4.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/8c00daf074f7.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/8c7a068bd709.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/8cfa1c1f593a.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/8d27baadbc81.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/8d8e505e2fb0.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/8e0f0d0871eb.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/8e973d12c8f6.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/8ef743c7d51b.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/8f345886c5b7.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/8f3b9c8b4099.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/8fd10a44c9e7.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/90d5c3c0031a.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/9142a942cfec.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/9283efdb1071.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/93d1bc903983.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/9410751ffaec.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/94834a259c25.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/94c98438e51f.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/954d732a4ce0.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/95d25ba97145.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/96e75af20936.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/97b11f8729f9.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/9840c3f32dcc.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/985d07294bee.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/98cec35dda63.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/99f0d559b29a.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/9a4c9cf73afb.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/9cafc65634fc.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/9e2fb734d90b.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/9ec036ae91b1.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/9ee3f4fe81c6.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/a12c6cb46374.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/a1f3f0cdbd7a.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/a2608f0e5668.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/a578fe8da82d.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/a5e41e5bff68.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/a61e0000f270.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/a64065ef252c.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/a7513bd4ba86.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/a78d8bf3d611.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/a825179860eb.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/a9afd2c39470.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/a9e6904ae9cb.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/ab8dfaeabaf4.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/abdd1f06bb03.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/acb4966a9895.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/ad49a558cd90.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/ad85dc9a6f7d.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/ade1a509f470.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/ae6be5c20c14.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/af256462c4f1.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/b04b6b23fc25.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/b129b94ff890.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/b12b17dd9850.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/b266676dca28.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/b478b0121e8b.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/b4b6d5d8d2f3.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/b4ecd80ef2b9.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/b52564463eca.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/b680b2adbe79.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/b9757dc3f0bd.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/b99455abcabb.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/b9e2c906f7a3.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/ba29779cb9ab.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/ba623bbcc2c4.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/bb084e4aff4c.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/bc5d59016950.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/bdac773ee6dc.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/bf0ca0f5a503.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/bf5954fb8498.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/bf61fb687aaf.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/bf8912d358f9.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/bfad8bc76040.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/bfb1465e4da9.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c070317af6b1.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c0b251dfac93.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c130eeeffb1c.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c1eaf30668de.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c2dfe1d7352d.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c347eb9a567f.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c425c7451ae6.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c8227dd6e32a.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c83e95ced4e8.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c84501d25d09.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c88bcda2cccb.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c89dbd1c9627.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c8a6e74f7ec6.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c8bb8bcbc852.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c953278032dd.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c9d4ccd42c66.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/c9e11c4251b2.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/cbc60ba16f78.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/cbeba889fead.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/cd12c1b8b4a4.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/ceb351cb5077.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/ceb5a373c5d9.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/cf05f96ca74d.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/cfd1b2c0e4ad.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/d06009323926.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/d1ae03dbd8d3.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/d2f046c59271.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/d31f3513674d.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/d3f326ca4d5b.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/d66f74919a99.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/d7235bbcdb57.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/d8b9d7552917.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/d957af79d16c.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/da19f5004423.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/da7898b5475e.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/dd3e0901ab54.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/ddba61f273f9.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/dea90ae9bccc.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/e051bb36f41a.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/e13e4583cd66.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/e1bd09e6575b.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/e2af262a2545.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/e44f5c192865.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/e474eec42d92.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/e5cc23e02b14.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/e68ec4518fd5.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/e6d1796665e0.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/e76c92843a11.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/e8f99bda3fd9.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/e91090a35892.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/e92be84e5dc3.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/e98f79bc5d5b.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/e9c19f5b17b5.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/ea27812c8c02.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/eca5b64850b4.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/ed10c73a17a1.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/ed1d5d86f6f1.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/f1aa463b9e5e.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/f2e949ee989f.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/f3816279dae1.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/f397fcff8f75.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/f3bb760167e8.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/f48a4d416913.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/f589f546fb8c.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/f617042d5969.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/f64207b16836.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/f6789d839643.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/f74f0b875a3d.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/f951dbeb7415.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/f99742bbc8d3.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/f9f119a65266.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/fb1bc172db7c.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/fb787c68a802.jsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/fdfa9686e9c6.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-target-blank/fe9b411eda3f.tsx` | `tests/lib/rules/jsx-no-target-blank.js` | derived |
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
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/104851b69942.tsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/182e0a8b5a9c.jsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/2c8612167df4.tsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/32f78cfc9e14.tsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/3e6ce54d8f0d.tsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/52316e0f2ee5.jsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/5437a4df9715.jsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/554ea3f51031.tsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/616ee1b3654f.tsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/63bc7886ec79.jsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/671d2aad5dce.jsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/698a7be06da6.tsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/69a6ed12079f.jsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/934224d6ec6d.jsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/950bcaf59738.tsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/9a73dc7dffc9.jsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/a2ba1b9c6487.jsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/ad18adc17339.jsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/b7888b393e0e.jsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/bc0532db0228.tsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/bf6241b6a126.jsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/d6b58c9a4724.tsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/e018d6d0a7be.tsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/e51c809bbffb.jsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/efce210d0350.jsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/efe561c7b91c.tsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/f011b1430fec.tsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/f0b9759a1a81.tsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
| `upstream/eslint-plugin-react/fixtures/jsx-no-undef/fbc63ee77fd3.jsx` | `tests/lib/rules/jsx-no-undef.js` | derived |
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
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/0023d4db171f.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/01c4aef77ae3.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/02e260fb8010.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/0719f854730a.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/0936b64b44b0.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/0b2474b0657f.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/0b6d2d35c0e4.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/0c54df17d7bd.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/0c653f381e68.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/0d0b8e775d40.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/0d36f59d003d.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/126c24b001af.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/1302b88b306a.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/13c94ba751b1.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/172fddd1c03b.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/176e211cf4f9.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/1bae2204613d.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/1c07f529fc45.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/1c9b48f25931.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/1e4344faf297.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/1ebd9a1d0c8a.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/1effa4ac695d.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/1fe1cdcbc8b5.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/200e7182a1ed.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/20e711542ee0.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/22b6c0dcd49c.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/22dd8d1ae418.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/230c5b31a217.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/231eb845ab97.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/2322259d7afd.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/23f5c0bb1d47.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/2403ed34f075.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/255617df40a0.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/26225076c906.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/26c817a2ef94.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/29104bb9809d.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/2937e6f74898.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/29ddd879d13c.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/2d3bbb30f033.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/2eb1596f4457.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/2f33ae9afc6d.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/307e611dc4a9.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/31397aca7786.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/317a9cf1a720.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/3417899e60ba.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/341e580e4794.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/37aa90a7eb7f.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/385dc8d28629.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/387eed7e114c.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/39bc59ae05ae.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/3af1e4d56110.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/3b760b8307d0.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/3eb200d847b6.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/41a442e138d1.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/47767b978111.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/49a414199d67.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/49dd0acd9c87.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/4b27466d9ebc.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/4b3877785c4b.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/4c9b0f33774a.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/4cc1f473766c.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/4e79467fd2f7.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/4f2dcba990ca.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/4f499dc584ca.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/4f4fef28bed0.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/5002b440ecb4.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/50328fce512d.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/541b8b45c460.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/547e9f42fceb.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/54918d9f20c6.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/55424fd298f3.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/5768300d08b3.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/5804231a20db.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/580fd0639711.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/586d8db80395.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/590a4ff655e4.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/5919a9b28d65.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/59c9d2f5800e.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/59e1085424c7.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/5a2e6a5c5dba.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/5a8591053ca8.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/5ba8f055b5e9.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/5c0cf889b571.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/5c3f6f832996.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/5c88d3f9967f.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/5d22eab70aae.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/5d661aed8e8e.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/6075d02616e3.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/61314252dc1c.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/621baf27f436.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/6374cc109347.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/640346a3ec97.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/646fe3fc0c04.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/676363d05b9c.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/68e2b932a862.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/693594a3dbf0.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/6a97dc845ea2.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/6aecc26ff0fb.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/6c91e135ea83.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/70200e41e70a.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/71654f963dd1.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/7322db37c52d.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/73a36ac9b769.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/751bf9370836.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/759d069d2ec0.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/765e0e2148d6.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/76edb1f8d732.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/79912dde902a.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/79ff2644fc1b.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/7a05f394747a.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/7c10b69af307.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/7d51e2d58b6f.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/7d95b52c4160.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/7daba0c2fcd8.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/7dfff313107c.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/7eac895cb1f2.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/7f0319ea3e1d.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/7fc40260cf48.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/7ffeceabd48a.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/8037eaddec36.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/80a913492677.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/80bb7390b242.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/8166917e76a3.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/85f940e1f54a.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/863e330324c6.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/874b81f770e1.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/877b4607b08e.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/88b3796c0c94.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/899ff5ed8bf4.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/8c935094f69c.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/902bb276fb9d.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/9037f3092349.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/9132b037f933.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/929b46fdb709.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/936e4a89c2e2.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/94a6cd0ef549.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/94b20d308e04.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/9635e157999f.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/9ae5e572aabc.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/9b3f05104ab2.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/9b4c9e9b5093.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/9b6776f28f40.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/9bd01cf2d596.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/9d671dd8983c.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/9fac9d97d9b8.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/9fce48171f47.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/a3b7a8fa70c9.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/a401c98c7401.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/a53928ae8e6c.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/a5d546102bb9.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/a87a3813efe2.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/b0a4f5f25fb1.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/b0e985a0bfd6.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/b179cb2c9f8b.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/b23ae21331e0.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/b23e756425cd.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/b27a1a442d27.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/b37abe88089d.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/b38981b240a7.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/b45e89f0174a.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/b5a0a6fa91ec.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/b6b8dcfc5ea9.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/b838d733ed5a.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/b959eadfb52d.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/be5b8f5a4d0f.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/be939ec95286.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/c04da2d1bc22.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/c0cf4ea6921d.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/c3dc57bcff5c.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/c7be3e3b4a82.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/c8e2b6f2fa63.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/ca6c88c169e9.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/cebf2ed04edc.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/cf0942b2f6f2.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/d0b71cb14759.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/d11bb797a7da.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/d22b9fe7e26b.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/d288d33e9882.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/d3814e77b35b.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/d5f3f267ec3c.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/d84241169053.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/dce603a26014.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/df409c80b7b9.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/df674834df3e.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/dff57c3734f2.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/e091753cd837.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/e0acc4675272.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/e466cab09a1b.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/e56ecb36d62f.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/e57fe4ff192a.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/e9bc9cb802fd.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/ea449340be43.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/ec3ee53c8949.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/eddb98ed43c5.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/edec6ac29a51.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/eed92d251626.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/eedb65cf794c.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/ef29e027f963.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/ef66426c7f15.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/f09e2c05b38a.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/f11ee9a704c6.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/f14aba7d1dda.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/f1f085137ca0.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/f58fa8b89c42.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/f5a6b5dd6618.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/f609fe1048bc.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/f60f3c71fee3.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/f630546a0825.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/fa4a5b5c057b.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/fadd3450b518.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/fbdcf63d665f.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/fc3aacf5e325.tsx` | `tests/lib/rules/no-array-index-key.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-array-index-key/ff0bdaf7ab2f.jsx` | `tests/lib/rules/no-array-index-key.js` | derived |
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
| `upstream/eslint-plugin-react/fixtures/no-children-prop/00e2d5e59f5c.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/0367ebaca1f4.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/0896a55cad02.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/08a9abf89a1f.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/09092c23e740.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/09c0481ff904.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/0a481e2846f7.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/0ab68f48a944.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/0aba866cfd74.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/0bd7805c69fc.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/0d95fd142e19.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/0e2347a08574.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/0ecc94acc528.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/109279127c6a.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/14972c9c84ef.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/17cf4e85da18.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/18d719647df5.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/1f1f2d54d26d.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/1f28d5d44b06.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/203dcfd227dc.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/2112607a2f20.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/22581ac82533.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/22bc589f53f3.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/23c0e0036323.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/252bd898fb9f.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/2745203d9875.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/277658000fee.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/27adbe9a99bc.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/297ea6763e5b.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/2aa0671b4aa1.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/2b7a92d96f2d.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/2c3de3acd11d.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/2e00ad0c511e.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/2f60ff7b1948.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/30be725c119e.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/31bc80a35cd6.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/31cabcd46377.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/32281a5ab14b.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/328f14045bf2.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/339eee93ee9b.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/341474d0a0fe.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/34f61dff03ab.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/35c04da6e0e8.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/3714404af473.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/389e0cd258cc.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/3af4bc1b918f.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/3c73f490d5f2.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/3da05f5ad8ec.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/3e310c6fec4e.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/3ecd323ea488.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/447c2aa0bbab.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/4519843f03d2.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/47014f970213.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/47457e507623.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/493ec6d85a42.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/4a0ab9a735b5.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/4a6e2bbb6c5f.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/4dc1a1f75f9c.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/4e1143fbc3e1.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/4e642e97d023.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/4f23a7173861.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/50e7b74319db.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/51045ba1b3d5.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/51717ad4d711.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/54558b3cd7bd.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/54dca81ca79d.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/555c1ad27c3c.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/5aa2a760d14f.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/5e3887946f40.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/612ad81a880b.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/61ed59b6836f.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/6264f305b4fa.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/626e58d63584.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/639675779dd8.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/66a688afade9.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/672a202b6359.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/67b644150fe6.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/68191a78bf18.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/6b4c11fe59a2.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/6bb18d806606.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/6cfc4a841a66.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/6e789ba836a7.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/6eb23c7c99eb.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/6f83770f6a33.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/70245df0eb50.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/703c2f70365a.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/704b48f5b87b.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/70509df6a664.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/71377062f210.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/71a901f130a2.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/73eddc93ac56.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/74f86efea322.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/7610169bad93.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/784bd4ba3540.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/79b8e028fb2d.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/7bb9c20a6c85.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/7db2b16a83cf.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/7e8a43fb86b0.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/804434d37654.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/80bb8c29cfcb.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/811ef2cb59f1.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/8352df219132.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/83f44d1c8ceb.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/8501f3e83874.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/8542f4e254f1.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/891be962cb0d.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/8a43c938eb63.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/8a4d0b96ba86.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/8a5916314d9d.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/8a8565ef061e.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/8cef16ce9a65.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/8ecdb24174fb.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/8effaff9abdb.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/9109c43482c0.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/917cda3dc920.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/91d7da7db24b.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/927dd45af629.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/93aa636270e4.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/945de716bcda.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/952b18fca425.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/958bbad6eb29.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/96662a395f07.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/980cbc51c78e.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/9968ae2693e3.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/9aa7490c2940.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/9c2b2736c0c2.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/9d889f02f92c.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/9e0f7237e7a9.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/9e5988b504f3.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/9f365ace701a.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/9f683990a628.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/9f93a055bcd9.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/9fb771da2a6c.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/a0ba0a6789e8.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/a12b6d3baacd.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/a2bbf1eaf45b.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/a380d34a89b2.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/a5dc6de5b5d0.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/a6d7424e4d13.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/a78bdc3b3299.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/a8270e35e1f8.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/a8dda4885dd8.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/aacce3533ff5.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/ac119db13373.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/ac314b442d00.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/ae30f256b407.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/b0ca581f4eca.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/b16e4031c879.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/b3f4d314defb.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/b4be2d0afb8f.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/b5fd4c201cfe.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/b60ca6c39ffb.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/b6fda7b3ebf8.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/b74b7a6603aa.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/bac0e6849983.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/c43036a2df71.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/c4df094d63af.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/c65f06c8ada9.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/c7720631386a.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/c7b4efa8fc1d.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/c937d4b63f21.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/cb61c7cd4343.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/cb6986eb3ae3.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/cd21d2f2ad3f.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/cdded0e1bfb2.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/cfe88139341c.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/d063541c3e07.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/d0a454b3f605.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/d3d65a219f6c.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/d3e4f52b3fb8.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/d58797539957.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/d5b0d4be472b.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/d6afe88426f5.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/dac818118282.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/dae78b96dec8.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/db1adc63cd80.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/dc01906bc7c4.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/dc68f214d401.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/dc6d6e59b9c2.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/dd0e57e14d1c.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/de8fad41a831.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/e20de65241bf.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/e48e1dcd18cf.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/e510c81a3887.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/e673684031c8.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/ebbaae2fa704.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/ec0063c99aad.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/ed64789350ed.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/edfc2d2e3ddd.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/eedcaeed3e4b.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/efc062e7ec96.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/f04b678e5282.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/f1aa3dfb6486.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/f4a00503365e.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/f4a1d126ae83.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/f86d8131c440.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/f89b944566ad.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/f8d965adabd9.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/fab3cd67b620.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/fcb7f8a42ad6.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/fce9007da3f6.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/fcea06d2b527.jsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/fd4a08c7ae27.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-children-prop/fdd20b0abe71.tsx` | `tests/lib/rules/no-children-prop.js` | derived |
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
| `upstream/eslint-plugin-react/fixtures/no-danger/05e4283a3b45.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/0616e94acdab.jsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/0dadd80a57ba.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/11d5f493f358.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/18299d82d4d1.jsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/19bb9fa1625c.jsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/20d08658bc17.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/221f77e976b0.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/22b3d99f83cf.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/2890f0c401cd.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/28dd8ca3da59.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/37650da74a61.jsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/4d74b781eff9.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/5275c19d2b75.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/528964f098d3.jsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/5445ee4e776f.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/5c76ed75ec00.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/5d651d4b2ffe.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/60fd51c3ee2e.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/64199704da54.jsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/834eab916044.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/8559e471e699.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/872144843d52.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/8bfcc0fe65b5.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/94cddb727c15.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/9990f0c175dc.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/a1d283376d98.jsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/abce28c09f39.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/af4fc3c19fea.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/b2085519d05e.jsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/b548bad4cea6.jsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/b63cfd69ebd0.jsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/d4a537cde3fd.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/e3af4b7546d4.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/f6df208d999b.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/fd1f044f6e7b.jsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/fe0408809ca0.tsx` | `tests/lib/rules/no-danger.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger/ff315e3579ed.jsx` | `tests/lib/rules/no-danger.js` | derived |
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
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/0187b056a54e.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/03f4c6d43f1a.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/068bc6ddd6f2.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/0eddf9d91fed.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/1325e48720f2.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/155dcbaa76e9.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/18004364a3d6.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/19cb55b8e877.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/19f9ee312ec7.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/1a1eb05aa77c.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/1da82e50ff78.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/1f28d5d44b06.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/25daebb23a4b.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/2715c09f03f6.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/2b7ac53d5042.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/2c2e92dbe7c4.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/3457389f2f5e.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/34b73c4cf6a5.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/368dcd08d6de.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/3a794c110618.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/3fd99d354acf.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/46ff388a372f.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/48bb9c97fce4.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/4ab6df3809d9.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/4ddad88105b8.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/51313acf5cbe.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/5ca5212c5ef0.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/5ec454e9d633.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/6153bd8b0a2e.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/690b7521f40f.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/6c5e94fd7cba.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/72541734f74a.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/7b7a9a0356d7.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/7bd3e161b0d9.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/7d860dad7669.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/803dfd7b619e.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/814d62f56242.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/826a1625970c.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/873b4ac723ce.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/87c569c7268e.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/87cc1e6d2f5c.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/882f11e5264d.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/8bf26d3a96c9.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/8c6af59e02ac.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/8ecdb24174fb.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/9617ac20eafd.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/991bdcbdc92d.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/9a0da9d285fb.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/9b7f403aed57.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/9b8c9a63e325.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/9bf8749a670d.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/9c931237a97a.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/9cd9dfdb67af.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/a169863bafde.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/a1d1b2f15d5d.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/a282b5cdc385.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/a2a84c5fc177.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/a34c9052706a.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/a4397294574f.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/a5396c7c296f.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/a5e16fcf2560.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/a8961be7b1ea.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/ac8c4a82e0ad.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/accd540fb777.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/ad8733ae45f3.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/ad99b9cd7167.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/b48e65b799cd.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/b5bad8873d8e.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/b74b7a6603aa.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/b9a2cff078e3.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/bf13de6366c1.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/bfb291633e32.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/c0c185bcb143.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/c2c6352b0396.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/c32d830808e2.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/c434f288a965.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/ca64b2099b48.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/cad724d9f6af.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/d10139126666.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/d101fabfce25.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/d1633e544460.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/d72775841fc6.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/dbc28e206e50.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/dea47c9131cf.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/eb0df8edccd0.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/ef9ac0e7a6d4.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/f247ae6ab3fb.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/f283c44c0cf9.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/f44551e91b85.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/f4cdc89e21bc.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/f629abe9a586.tsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/f972c43bf1e5.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-danger-with-children/feccc3e2e72a.jsx` | `tests/lib/rules/no-danger-with-children.js` | derived |
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
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/025fa77b8689.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/060fa55ca57b.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/16632536a213.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/16e003be448e.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/17b0253e0eb8.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/1a32d4ade7dc.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/1d14c9b1aa10.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/258335320a03.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/2a3285c215b2.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/2ac0d02c98d4.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/2b81bb75df2a.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/2be8c515af42.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/2d1c56bc4d58.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/33b071df06fe.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/33d1471f014e.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/37dd62dcdbea.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/392c08d5ac6d.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/3bd7d13ea5ac.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/3bf626fe7493.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/3dc02b7b6a8e.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/3dd12961343a.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/4021b10699c6.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/4740bbde968a.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/4d0d018a9c4a.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/50567a868811.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/5668f2a83a6b.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/5d82916b6430.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/5e3fbee996b0.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/614751907e91.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/6917e7aa5ffe.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/6af9f1a9eb0a.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/6d12e53e6011.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/71e02ad358ea.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/7d3e9078e56c.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/7e8ce0430d3e.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/7f50c485f820.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/889ae3553f9b.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/8969d0ba23a3.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/a226cf0f136d.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/a39c14eab1ca.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/a6ea68db0a75.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/b2a8ce807edf.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/b7e7f3297c8f.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/ba324be578a3.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/ba6bd496a525.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/c281be161f21.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/c40c89c2ab5f.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/c598209eab22.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/cd0801e60c33.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/d4aa833d6be6.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/da5fc8422af3.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/dad1d538ee4d.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/db2043b4d4c5.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/db607ce65e6c.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/dcfafabd3468.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/dd62fc0a8980.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/e01d197863b5.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/e58835f5f571.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/ec86e53420ae.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/f12d3e08510d.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/f223a876764d.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/f7d9fdb886a9.jsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-direct-mutation-state/fca4709300a3.tsx` | `tests/lib/rules/no-direct-mutation-state.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/valid-000.jsx` | `tests/lib/rules/no-find-dom-node.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/valid-001.jsx` | `tests/lib/rules/no-find-dom-node.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/valid-002.jsx` | `tests/lib/rules/no-find-dom-node.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/valid-003.jsx` | `tests/lib/rules/no-find-dom-node.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/invalid-000.jsx` | `tests/lib/rules/no-find-dom-node.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/invalid-001.jsx` | `tests/lib/rules/no-find-dom-node.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/invalid-002.jsx` | `tests/lib/rules/no-find-dom-node.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/invalid-003.jsx` | `tests/lib/rules/no-find-dom-node.js invalid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/0e1a6cb41203.jsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/1e901d66c2e0.jsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/2bbeb638e062.tsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/2ed9b56c1841.tsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/40585b555f08.tsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/4096983e61f8.tsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/553307e7a61a.tsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/5accfb6531e1.tsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/6fd04788760c.tsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/772f1d7e878e.tsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/7aae4a7eefea.jsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/7b5525159e64.jsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/807d4eed27c9.tsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/84cbb7a62bf7.jsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/889c0244f832.tsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/89ec731c3d17.tsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/ae3aa0f32ee8.tsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/b21f8266fafd.jsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/b5f637c72412.tsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/dc4059ee42ed.tsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/e56defad4e33.jsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/f3ce8e9b5a0b.tsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/f401beb35ba8.jsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-find-dom-node/fceb09c8e48d.tsx` | `tests/lib/rules/no-find-dom-node.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/valid-000.jsx` | `tests/lib/rules/no-is-mounted.js valid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/valid-001.jsx` | `tests/lib/rules/no-is-mounted.js valid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/valid-002.jsx` | `tests/lib/rules/no-is-mounted.js valid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/valid-003.jsx` | `tests/lib/rules/no-is-mounted.js valid[3]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/invalid-000.jsx` | `tests/lib/rules/no-is-mounted.js invalid[0]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/invalid-001.jsx` | `tests/lib/rules/no-is-mounted.js invalid[1]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/invalid-002.jsx` | `tests/lib/rules/no-is-mounted.js invalid[2]` | extracted (its `code`, verbatim) |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/09cf76902f33.tsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/0e1a6cb41203.jsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/0fd92b6170f5.tsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/27722c56d056.tsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/36352185cb15.jsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/37ad3570d18c.tsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/4475c7383209.tsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/4b13910c88e9.jsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/4dbe0319815b.jsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/57d61127ad36.tsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/6f7d2d5f621b.jsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/807d4eed27c9.tsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/89ec731c3d17.tsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/941086aa1b31.tsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/9bb9fb11cd02.tsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/b0ed10ccf143.jsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/b49ab005480c.jsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/b772b530ee50.tsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/df8a08d1ac59.tsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/ec3afe8db3d6.tsx` | `tests/lib/rules/no-is-mounted.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-is-mounted/f07d481da54e.tsx` | `tests/lib/rules/no-is-mounted.js` | derived |
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
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/0b6a63117c82.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/14cfc13261f4.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/20bb7f51a5b0.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/23a781932be4.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/2506989174a4.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/252ab8eaf786.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/2800f21f6b3a.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/2d2de3a0c877.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/309077ab1b11.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/355d21b44f7a.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/38169917ab32.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/43d3014c7769.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/4ce4c195d37e.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/4d3ba3853b71.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/4fa6e2a9b9cd.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/5537d6b22ae8.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/55a8b4312b4c.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/5a9f7aa93e71.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/5c117c2fe44b.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/63f27a35ea46.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/662bd6c3a5b9.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/66aefb135603.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/6780b8ee14c2.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/722f78b60fe5.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/73ad73cede4e.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/73e85b1a3317.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/770b437b04b7.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/7941161f78da.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/7e5e086a077b.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/7fb0ba7b806c.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/813e5d053465.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/88a290924998.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/950339e6d263.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/9efa9234e23e.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/b50e33344781.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/bde58b49dbb9.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/bdff8a4bce14.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/c482c8a49894.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/ccbd5ddf32f5.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/ceb7f152ed63.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/cfc938554db8.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/d29e1ea3dec1.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/da774a8600ae.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/db6b8240d9a4.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/db87d558b764.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/e1b5a49e020f.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/ed2d5a77f13b.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/f13d1e941209.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/f4c69c59e753.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/f8dc9511d1a2.jsx` | `tests/lib/rules/no-render-return-value.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-render-return-value/fe57ff0853d0.tsx` | `tests/lib/rules/no-render-return-value.js` | derived |
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
| `upstream/eslint-plugin-react/fixtures/no-string-refs/0656691dad43.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/0ad953117567.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/10c58c15b2f3.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/2caa275de0db.jsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/2eef2c13c192.jsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/31f172cf70d0.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/327e46fd1417.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/3a78517d3eb9.jsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/3d36aa9a42c6.jsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/3e685705ff94.jsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/487a5d426e58.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/5110907a6e09.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/5c57a4a17ed2.jsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/663fb38a191f.jsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/69f6375d4fec.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/773af7b4834d.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/78d3de25d7f4.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/80262996fb9d.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/818061952f35.jsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/8ea5bc29c32c.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/912a5bbdf7e6.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/96e87ec30052.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/973040efd652.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/97e7b95700ec.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/a890778de2b6.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/adcd9d8dfbfc.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/b8e4a9e2e80d.jsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/b94faa2f05f4.jsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/cfb1a12a2a8c.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/d0e1e2982a3b.jsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/ec7ca5d11f66.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/f1d322fb025c.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-string-refs/f1d3758e5181.tsx` | `tests/lib/rules/no-string-refs.js` | derived |
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
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/1104874ef653.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/157e85777c87.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/1848aa90a478.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/19d7bc0eb973.jsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/2aa26a9622e8.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/2d6b14414042.jsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/301edbed2cc6.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/3b60e6839acb.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/45c57585d9b5.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/48741d341f9a.jsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/49888d225bf4.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/4e251cf1f6d5.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/51a312f06db8.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/51acc22c3729.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/5e9958acb63f.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/72de2f97b6a3.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/76e211d1ef96.jsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/791cd6ef8ab2.jsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/7dd9bd1cf876.jsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/859e0c810d96.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/8b26b909bf82.jsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/8dc525ba3687.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/9a2b24c7390f.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/a8c8fa9fb620.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/ae0eee9be835.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/b8da440cf11d.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/bdb7fe2412a8.jsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/c0d45114a32e.jsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/c5143d16ba3b.jsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/d06ad0a87257.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/d1ed2999de21.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/da45ac36a44e.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/e221bf1f61f2.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/e38da5e8a958.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/edab46c58e19.jsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/f78c16a0d8cb.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/fce88dafea81.jsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/fe88450dd26a.jsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
| `upstream/eslint-plugin-react/fixtures/no-unescaped-entities/ff4015073102.tsx` | `tests/lib/rules/no-unescaped-entities.js` | derived |
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
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/018db0278d49.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/044d12bbbd99.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/0aa7dcaa5ff1.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/0ae999ecc0ff.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/0c7b7c88bcb1.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/1664e46d21eb.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/1dac82805969.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/1e8f840fd98e.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/2d3dc5203639.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/2f34a18906cb.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/433371a4c105.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/485e11565169.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/49a21a5043fc.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/56f072680e53.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/5b434732a567.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/5b4949404e30.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/5c01d66d5893.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/5d91cb64683b.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/611a6bc3b70b.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/643a2a729a84.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/6e0416932a07.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/7315f3d0faf0.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/742aca9d4bf0.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/828762eb2d1f.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/85e9aaaf6fc3.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/8668aebd9d2e.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/86f9edb853de.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/8c1f9a8ca191.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/991a8909fab5.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/9bbcea66d055.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/9f4821d9c8de.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/a3238812e92c.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/accdfad39247.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/bf8dba37244b.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/c1a25bc09d9d.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/c1edf0057570.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/c36ed54523d6.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/c5b38429a802.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/c956362a82c2.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/c9b849abbee1.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/ce5b486a11ba.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/d5b43b2623fa.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/e311cd4997c7.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/e31ea3f5c02d.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/e37e10ee6807.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/e4fcac134522.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/ea615948fb8e.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/f388cfef5a49.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/f59e79020eaf.tsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
| `upstream/eslint-plugin-react/fixtures/react-in-jsx-scope/f89afb0c9593.jsx` | `tests/lib/rules/react-in-jsx-scope.js` | derived |
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
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/01ae92cf9bc4.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/060aa19b2831.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/06662b73a65a.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/09213e62631d.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/0ba4ea1a15f2.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/0c2b6fcb69c0.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/0c4e4a0d6fec.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/0e97622c724a.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/0f5bdd5a8537.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/0f895c807d97.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/11aeae52eb7f.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/1653f4807e27.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/19a16468b681.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/1d9128c25ac1.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/1dd49fd39545.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/1e34c22e57d8.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/1f19e06b5643.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/1f6cd820efd0.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/1fa7c55d63ee.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/21ade3d6cb94.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/27143e12d413.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/2751212597f6.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/292c7743b441.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/29ba376fb8ba.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/2d67a548efcb.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/2fb334177e95.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/30252240f3ba.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/30fa0de28b5f.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/327d7c8a9f40.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/34234232010c.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/3b339564e402.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/3dd2b749edb1.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/40624f56fa40.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/42ee26aa4af0.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/46b97bb72c91.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/4821cb99dd81.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/482a34237bc2.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/4bae5156213d.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/4bb8728ebff2.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/4c2984213f7f.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/4c54e06d41b0.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/4f749166196f.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/5220c2e8f9ab.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/52220c642176.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/52bbd224e44a.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/55e03f93b1ac.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/56ab3447bcf2.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/581072aefbda.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/59e453fc22c5.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/5bfc1ffb02fa.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/5c35f1dd1bea.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/5d3e4482a18f.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/5ee0c9888d21.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/60076d8ffe91.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/62ff0caae46b.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/65f276c5e9af.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/6619ea8acffd.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/6806ab49a175.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/68865dc9cee6.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/6e0a0e247e2a.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/70cd9a310c43.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/7144625cda51.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/751ad8a9dfe2.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/75ad105492f0.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/76090f11abb6.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/7716f44720b6.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/7748f432d0cb.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/78ffd985783a.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/799ea1da4bd0.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/7a9c9c9519af.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/7c5ffea2ab59.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/7d372cce1a96.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/7da02e046311.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/85c28243e1a7.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/868c0a96fbd0.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/8799624e2a96.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/8832f02b7ece.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/893e8f2eb7f5.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/8d337ec50856.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/8d85cd2b2268.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/8ee9c9f4ea12.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/90d5927c808b.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/91ea306f1057.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/92c7102a3119.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/95939370d014.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/98e8f68b94ad.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/9afbeba15da9.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/9c1db43e09d6.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/9d5e27f9b481.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/9e4c282abfdf.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/9e8bfa797ed9.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/a2d8ea77ddee.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/a4571af6b03f.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/a6422f4aca4e.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/a703d09079fe.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/a975a255875f.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/a9b4fd46a182.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/aaa17f1980ff.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/aca233be634a.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/acbb72e53cb9.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/ae2ee29473f5.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/af8e7bb6e53b.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/b047d233f571.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/b1119c8d227f.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/b1753f6925c5.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/b34d7cb286ce.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/b65c689d6e2c.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/b67e7f066948.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/bf35b41c5194.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/bfb24730ae4b.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/c27fb029ad8d.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/c3a059187ce7.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/cb353ce2b50e.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/ce383fc3b8fa.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/cf1e3e553393.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/d41dc58695b5.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/d51f747bebfe.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/d669187870f0.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/dd62d182d49f.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/de10217699c2.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/de3240f890a2.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/e17930678743.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/e2f6c07cade3.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/e319d43a72a2.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/e84726f01026.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/e926e2249e95.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/e942a925aa26.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/ee4340d14be5.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/f09ab39cf6b9.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/f1766526c69d.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/f1a092c18249.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/f25ecb47f053.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/f26818d57034.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/f31fa6a65321.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/f3ba9069a2b7.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/f45ba3b874c7.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/f6611dcfe87a.jsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/f8eea3321f70.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/f8f62e9166e8.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/f9351dbc12cc.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
| `upstream/eslint-plugin-react/fixtures/self-closing-comp/fef31af34fd8.tsx` | `tests/lib/rules/self-closing-comp.js` | derived |
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
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/01617356ffc7.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/016a4b5a2a82.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/03db71f69156.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/03e6d6e5624b.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/0b593185fc50.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/0ecfeb92c295.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/1423e90181d2.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/174f9bf8f790.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/20416d4c8e7c.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/2776e8b1c4c6.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/291ce1e5dfe3.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/2b6b268e47dd.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/2d47142d1eb8.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/2eced1b63e9a.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/38405a019e1e.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/392bbcc486a7.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/3999c8735b2e.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/3b8ec5ed4f94.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/3c348852fa1a.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/425fab37bb38.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/43a3c8b673a8.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/49d294b88ace.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/4c2d3b60c9b4.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/4cf17eef3183.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/4eeb7226c5ca.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/56e5420ff34f.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/579949b93948.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/5b8c0fc9e716.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/5bf63de128d7.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/5ea29635f6fa.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/6004d095f29b.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/604cdfef51da.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/60d08ed582f7.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/61462756e9e2.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/628cd8ddbaf2.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/63967b73c6a2.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/6891ceff722a.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/6a7377483080.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/6b37db576849.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/6cc3de542b7a.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/6ee7430ce747.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/70e77d23846c.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/72fad178d30a.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/801e4272fdb7.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/80901c6575ce.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/80e36ee5b765.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/81e8a6c74a12.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/86f216c21cb9.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/87d5d1009af5.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/8d38d48e3e33.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/9b65f51c8dcd.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/9bae48819264.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/a774c3ca67fa.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/a883b647292a.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/adf9619a103c.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/b0f67e12c07b.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/ba4436c76607.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/c218b682dbb3.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/cb182e5120db.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/cd5c33c113e9.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/d06136afbcee.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/d47acebcd2a8.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/d62440e22b1e.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/d9eb41a41c4e.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/dfa260978387.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/e441a9b0b911.jsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/e92ad174aa1c.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/ea5099a9ad34.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/f2cf25f5e15d.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/f6ce6116df2b.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/f76bbef45753.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |
| `upstream/eslint-plugin-react/fixtures/void-dom-elements-no-children/f87a7db315a4.tsx` | `tests/lib/rules/void-dom-elements-no-children.js` | derived |

> `recording.json` is a port readiness recording, and no file of this pack, so it is recorded here rather than in the
> table above: the tool-pack's `recording.lock` names it and a GitHub release holds the bytes (docs/port-readiness.md).
> It carries excerpts of this upstream's tests (`tests/lib/rules/*.js`: the `code` of each RuleTester case),
> redistributed under the licence above, together with Rerail's Apache-2.0; `LICENSE` beside this file is this
> upstream's own, at the pinned commit.

