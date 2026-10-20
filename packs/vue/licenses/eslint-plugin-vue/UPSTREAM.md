# eslint-plugin-vue 10.11.1

Upstream: [vuejs/eslint-plugin-vue](https://github.com/vuejs/eslint-plugin-vue/tree/ec9c53a6fbed637d7b0fbcd6893b0206ae32839f) at tag `v10.11.1` (commit `ec9c53a6fbed637d7b0fbcd6893b0206ae32839f`). License: MIT (Copyright (c) 2017 Toru Nagashima); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at that tag.

The files listed below are the code of RuleTester cases of the rules the Vue pack ports (`../../conformance.json`), and remain under eslint-plugin-vue's license. Each is one case's `code`, copied verbatim, or wrapped in a `<script>` element when the case is a script (a non-`.vue` filename, a parser other than vue-eslint-parser, or code with no SFC block and no filename), so that it is a `.vue` file. Every other file under `upstream/` is Rerail's, under the Apache License 2.0 (this repository's `LICENSE`): `read_tests.mjs`, `generate.mjs`, `authored.mjs`, `cases.json`, the fixtures `rerail-*.vue` (written by Rerail; `upstream/eslint-plugin-vue/UPSTREAM.md` lists them) and the recordings they write by running ESLint 10.11.0 with eslint-plugin-vue 10.11.1.

This file is written by `upstream/eslint-plugin-vue/generate.mjs fixtures`. Paths are relative to the pack directory (`packs/vue/`).

| File | Upstream case | Copy |
|---|---|---|
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/valid-001.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/valid-002.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/valid-003.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `valid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/valid-004.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `valid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/valid-005.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `valid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/valid-006.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `valid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/valid-007.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `valid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/valid-008.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `valid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/valid-009.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `valid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/valid-010.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `valid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/valid-011.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `valid/11` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/valid-012.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `valid/12` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/valid-013.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `valid/13` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/valid-014.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `valid/14` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/valid-015.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `valid/15` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/valid-016.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `valid/16` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/invalid-000.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/invalid-001.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/invalid-002.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `invalid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/invalid-003.vue` | `tests/lib/rules/require-v-for-key.test.ts` case `invalid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-v-for-key/e3b0c44298fc.vue` | `tests/lib/rules/require-v-for-key.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/valid-004.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` case `valid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/valid-005.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` case `valid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/valid-006.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` case `valid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/valid-010.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` case `valid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/valid-011.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` case `valid/11` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/valid-012.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` case `valid/12` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/invalid-000.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/invalid-001.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/invalid-002.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` case `invalid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/invalid-003.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` case `invalid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/invalid-004.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` case `invalid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/invalid-005.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` case `invalid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/invalid-006.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` case `invalid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/invalid-007.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` case `invalid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/invalid-008.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` case `invalid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/invalid-009.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` case `invalid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/0623a54389c4.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/54c6994994ab.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/5e333537cf67.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/ce6df5384956.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/d80387da56c3.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/dd81809f9a2e.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-use-v-if-with-v-for/e3b0c44298fc.vue` | `tests/lib/rules/no-use-v-if-with-v-for.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-duplicate-attributes/valid-001.vue` | `tests/lib/rules/no-duplicate-attributes.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-duplicate-attributes/valid-002.vue` | `tests/lib/rules/no-duplicate-attributes.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-duplicate-attributes/valid-003.vue` | `tests/lib/rules/no-duplicate-attributes.test.ts` case `valid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-duplicate-attributes/valid-004.vue` | `tests/lib/rules/no-duplicate-attributes.test.ts` case `valid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-duplicate-attributes/valid-005.vue` | `tests/lib/rules/no-duplicate-attributes.test.ts` case `valid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-duplicate-attributes/valid-006.vue` | `tests/lib/rules/no-duplicate-attributes.test.ts` case `valid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-duplicate-attributes/valid-007.vue` | `tests/lib/rules/no-duplicate-attributes.test.ts` case `valid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-duplicate-attributes/valid-008.vue` | `tests/lib/rules/no-duplicate-attributes.test.ts` case `valid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-duplicate-attributes/invalid-000.vue` | `tests/lib/rules/no-duplicate-attributes.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-duplicate-attributes/invalid-001.vue` | `tests/lib/rules/no-duplicate-attributes.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-duplicate-attributes/89f3aefb2608.vue` | `tests/lib/rules/no-duplicate-attributes.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-duplicate-attributes/ae2afce7d2bb.vue` | `tests/lib/rules/no-duplicate-attributes.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-duplicate-attributes/e3b0c44298fc.vue` | `tests/lib/rules/no-duplicate-attributes.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-v-html/valid-001.vue` | `tests/lib/rules/no-v-html.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-html/valid-002.vue` | `tests/lib/rules/no-v-html.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-html/valid-003.vue` | `tests/lib/rules/no-v-html.test.ts` case `valid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-html/invalid-000.vue` | `tests/lib/rules/no-v-html.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-html/invalid-001.vue` | `tests/lib/rules/no-v-html.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-html/invalid-002.vue` | `tests/lib/rules/no-v-html.test.ts` case `invalid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-html/4fd7bf847169.vue` | `tests/lib/rules/no-v-html.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-v-html/50a492878e71.vue` | `tests/lib/rules/no-v-html.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-v-html/ab4137c1d3e3.vue` | `tests/lib/rules/no-v-html.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-v-html/e3b0c44298fc.vue` | `tests/lib/rules/no-v-html.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/valid-000.vue` | `tests/lib/rules/no-child-content.test.ts` case `valid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/valid-001.vue` | `tests/lib/rules/no-child-content.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/valid-002.vue` | `tests/lib/rules/no-child-content.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/valid-003.vue` | `tests/lib/rules/no-child-content.test.ts` case `valid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/valid-004.vue` | `tests/lib/rules/no-child-content.test.ts` case `valid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/valid-005.vue` | `tests/lib/rules/no-child-content.test.ts` case `valid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/valid-006.vue` | `tests/lib/rules/no-child-content.test.ts` case `valid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/valid-008.vue` | `tests/lib/rules/no-child-content.test.ts` case `valid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/invalid-000.vue` | `tests/lib/rules/no-child-content.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/invalid-001.vue` | `tests/lib/rules/no-child-content.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/invalid-002.vue` | `tests/lib/rules/no-child-content.test.ts` case `invalid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/invalid-003.vue` | `tests/lib/rules/no-child-content.test.ts` case `invalid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/invalid-004.vue` | `tests/lib/rules/no-child-content.test.ts` case `invalid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/invalid-005.vue` | `tests/lib/rules/no-child-content.test.ts` case `invalid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/4dc325877298.vue` | `tests/lib/rules/no-child-content.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/8681e5eae470.vue` | `tests/lib/rules/no-child-content.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-child-content/c25a89ea57a6.vue` | `tests/lib/rules/no-child-content.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-deprecated-v-on-native-modifier/valid-000.vue` | `tests/lib/rules/no-deprecated-v-on-native-modifier.test.ts` case `valid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-deprecated-v-on-native-modifier/valid-001.vue` | `tests/lib/rules/no-deprecated-v-on-native-modifier.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-deprecated-v-on-native-modifier/valid-002.vue` | `tests/lib/rules/no-deprecated-v-on-native-modifier.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-deprecated-v-on-native-modifier/valid-003.vue` | `tests/lib/rules/no-deprecated-v-on-native-modifier.test.ts` case `valid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-deprecated-v-on-native-modifier/valid-004.vue` | `tests/lib/rules/no-deprecated-v-on-native-modifier.test.ts` case `valid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-deprecated-v-on-native-modifier/invalid-000.vue` | `tests/lib/rules/no-deprecated-v-on-native-modifier.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-deprecated-v-on-native-modifier/invalid-001.vue` | `tests/lib/rules/no-deprecated-v-on-native-modifier.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-001.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-002.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-003.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-004.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-005.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-006.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-007.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-008.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-009.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-010.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-011.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/11` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-012.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/12` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-013.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/13` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-014.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/14` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-015.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/15` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-016.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/16` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-017.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/17` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-018.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-019.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/19` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-020.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/20` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-021.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/21` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-022.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/22` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-023.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/23` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-024.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/24` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-025.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/25` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-026.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/26` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-027.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/27` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-028.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/28` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-029.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/29` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-030.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/30` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/valid-031.vue` | `tests/lib/rules/valid-v-for.test.ts` case `valid/31` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-000.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-001.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-002.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-003.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-004.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-005.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-006.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-007.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-008.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-009.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-010.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-011.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/11` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-012.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/12` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-013.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/13` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-014.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/14` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-015.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/15` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-016.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/16` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-017.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/17` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-018.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-019.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/19` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-020.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/20` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-021.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/21` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/invalid-022.vue` | `tests/lib/rules/valid-v-for.test.ts` case `invalid/22` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/valid-v-for/e3b0c44298fc.vue` | `tests/lib/rules/valid-v-for.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-v-text-v-html-on-component/valid-000.vue` | `tests/lib/rules/no-v-text-v-html-on-component.test.ts` case `valid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-text-v-html-on-component/valid-001.vue` | `tests/lib/rules/no-v-text-v-html-on-component.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-text-v-html-on-component/valid-002.vue` | `tests/lib/rules/no-v-text-v-html-on-component.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-text-v-html-on-component/valid-005.vue` | `tests/lib/rules/no-v-text-v-html-on-component.test.ts` case `valid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-text-v-html-on-component/invalid-000.vue` | `tests/lib/rules/no-v-text-v-html-on-component.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-text-v-html-on-component/invalid-001.vue` | `tests/lib/rules/no-v-text-v-html-on-component.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-text-v-html-on-component/invalid-002.vue` | `tests/lib/rules/no-v-text-v-html-on-component.test.ts` case `invalid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-text-v-html-on-component/invalid-003.vue` | `tests/lib/rules/no-v-text-v-html-on-component.test.ts` case `invalid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-text-v-html-on-component/invalid-004.vue` | `tests/lib/rules/no-v-text-v-html-on-component.test.ts` case `invalid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-text-v-html-on-component/invalid-005.vue` | `tests/lib/rules/no-v-text-v-html-on-component.test.ts` case `invalid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-text-v-html-on-component/invalid-007.vue` | `tests/lib/rules/no-v-text-v-html-on-component.test.ts` case `invalid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-text-v-html-on-component/04b86739eacd.vue` | `tests/lib/rules/no-v-text-v-html-on-component.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-v-text-v-html-on-component/44c0bad619c8.vue` | `tests/lib/rules/no-v-text-v-html-on-component.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-v-text-v-html-on-component/94fb9724b739.vue` | `tests/lib/rules/no-v-text-v-html-on-component.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-v-text-v-html-on-component/ef45ed55fae6.vue` | `tests/lib/rules/no-v-text-v-html-on-component.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/valid-000.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` case `valid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/valid-001.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/valid-002.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/valid-003.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` case `valid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/invalid-000.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/invalid-001.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/invalid-003.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` case `invalid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/invalid-005.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` case `invalid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/4ffa94f1b021.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/9d9ad727b849.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/c7058b1e9e1f.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/d4097972aa9a.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/d55d72dbad23.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/f84b5c878244.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/valid-001.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/valid-002.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/valid-006.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `valid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/valid-008.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `valid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/valid-010.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `valid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/valid-012.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `valid/12` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/invalid-001.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/invalid-003.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `invalid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/invalid-005.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `invalid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/invalid-006.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `invalid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/invalid-011.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `invalid/11` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/invalid-012.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `invalid/12` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/invalid-016.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `invalid/16` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/invalid-017.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `invalid/17` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/invalid-018.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `invalid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/invalid-019.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `invalid/19` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/invalid-020.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `invalid/20` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/invalid-021.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `invalid/21` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/invalid-023.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` case `invalid/23` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/00504a1c4a6b.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/02469cae35d1.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/0e9b887a33e6.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/133f00e74cb1.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/15fd5d58a3c2.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/18352b312626.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/225a343f0360.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/3118327c4b0a.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/6a3b15d0d511.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/6b0fc478afde.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/7e7eb22aee8a.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/882c977a7b39.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/ad49728f3700.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/bdb5495f7594.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/cb24479bfa3d.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/d7e769ad2f2f.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/dd2efbdfdd47.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/e3b0c44298fc.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/e3b8c9509f22.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/e9038288e690.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/f49d7337981b.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/f6f7842f8484.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/attribute-hyphenation/fba60f7315d6.vue` | `tests/lib/rules/attribute-hyphenation.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-000.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-001.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-002.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-003.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-004.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-005.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-006.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-007.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-008.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-009.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-010.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-011.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/11` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-012.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/12` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-013.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/13` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-014.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/14` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-015.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/15` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-016.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/16` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-017.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/17` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-018.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-019.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/19` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-020.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/20` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-021.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/21` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-022.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/22` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-023.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/23` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-024.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/24` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-025.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/25` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-026.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/26` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-027.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/27` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-028.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/28` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-029.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/29` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-030.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/30` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-031.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/31` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/valid-032.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `valid/32` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-000.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-001.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-002.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-003.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-004.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-005.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-006.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-007.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-008.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-009.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-010.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-011.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/11` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-012.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/12` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-013.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/13` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-014.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/14` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-015.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/15` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-016.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/16` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-017.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/17` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-018.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-019.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/19` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-020.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/20` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-021.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/21` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-022.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/22` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-023.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/23` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-024.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/24` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-025.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/25` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-026.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/26` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-027.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/27` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-028.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/28` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-029.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/29` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-030.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/30` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-031.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/31` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-032.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/32` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-033.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/33` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-034.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/34` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-035.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/35` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-036.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/36` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-037.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/37` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-038.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/38` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-039.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/39` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-040.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/40` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-041.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/41` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-042.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/42` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-043.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/43` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-044.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/44` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-045.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/45` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-046.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/46` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-047.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/47` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-048.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/48` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-049.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/49` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-050.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/50` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-051.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/51` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-052.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/52` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-053.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/53` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-054.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/54` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-055.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/55` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-056.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/56` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-057.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/57` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-058.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/58` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-059.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/59` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-060.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/60` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-061.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/61` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-062.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/62` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-v-else-if/invalid-063.vue` | `tests/lib/rules/no-dupe-v-else-if.test.ts` case `invalid/63` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-000.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-001.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-002.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-003.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-004.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-005.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-006.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-007.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-008.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-009.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-013.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/13` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-014.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/14` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-015.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/15` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-016.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/16` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-017.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/17` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-018.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-019.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/19` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/valid-020.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `valid/20` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-000.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-001.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-002.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-003.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-004.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-005.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-006.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-007.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-008.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-009.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-014.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/14` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-016.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/16` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-017.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/17` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-018.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-019.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/19` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-020.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/20` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-021.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/21` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-022.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/22` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-023.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/23` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/invalid-024.vue` | `tests/lib/rules/no-unused-vars.test.ts` case `invalid/24` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/30a56badf909.vue` | `tests/lib/rules/no-unused-vars.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/40e861189a9c.vue` | `tests/lib/rules/no-unused-vars.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/8a1c56f1295f.vue` | `tests/lib/rules/no-unused-vars.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/ef49b9501e1f.vue` | `tests/lib/rules/no-unused-vars.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/f211b6a004e7.vue` | `tests/lib/rules/no-unused-vars.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-unused-vars/fe3a6fc2fba8.vue` | `tests/lib/rules/no-unused-vars.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/valid-001.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/valid-002.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/valid-003.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `valid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/valid-004.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `valid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/valid-005.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `valid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/valid-006.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `valid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/valid-007.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `valid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/valid-008.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `valid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/valid-009.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `valid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/valid-010.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `valid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/valid-011.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `valid/11` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/valid-012.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `valid/12` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/invalid-000.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/invalid-001.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/invalid-002.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `invalid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/invalid-003.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `invalid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/invalid-004.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `invalid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/invalid-005.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `invalid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/invalid-006.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `invalid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/invalid-007.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `invalid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/invalid-008.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `invalid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/invalid-009.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `invalid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/invalid-010.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `invalid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/invalid-011.vue` | `tests/lib/rules/no-template-shadow.test.ts` case `invalid/11` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/96a3a1cd166a.vue` | `tests/lib/rules/no-template-shadow.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-template-shadow/e3b0c44298fc.js` | `tests/lib/rules/no-template-shadow.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-000.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-001.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-002.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-003.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-004.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-005.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-006.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-007.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-008.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-009.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-010.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-011.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/11` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-012.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/12` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-013.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/13` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-014.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/14` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-015.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/15` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-016.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/16` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-017.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/17` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-018.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-019.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/19` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/valid-020.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `valid/20` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/invalid-000.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/invalid-001.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/invalid-002.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `invalid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/invalid-003.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `invalid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-unused-refs/invalid-004.vue` | `tests/lib/rules/no-unused-refs.test.ts` case `invalid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-000.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/0` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-001.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-002.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-003.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/3` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-004.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/4` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-005.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/5` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-006.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/6` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-007.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/7` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-008.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/8` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-009.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/9` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-010.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/10` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-011.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/11` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-012.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/12` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-013.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/13` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-014.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/14` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-015.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/15` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-016.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/16` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-017.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/17` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-018.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-019.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/19` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-020.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/20` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-021.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/21` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-022.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/22` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-023.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/23` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-024.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/24` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-025.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/25` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-026.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/26` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/valid-027.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `valid/27` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-000.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/0` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-001.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-002.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-003.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/3` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-004.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/4` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-005.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/5` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-006.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/6` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-007.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/7` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-008.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/8` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-009.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-010.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-011.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/11` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-012.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/12` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-013.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/13` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-014.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/14` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-015.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/15` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-016.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/16` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-017.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/17` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-018.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/invalid-019.vue` | `tests/lib/rules/no-ref-as-operand.test.ts` case `invalid/19` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/27f266049d83.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/2d63300ed8b2.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/336fd17f3d08.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/51fce401e969.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/52a378b40051.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/5aa6ea317cce.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/7f7f2a365ded.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/9218219971ce.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/9f1b3ac4fbf8.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/aac0779dd072.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/b1c783b1d9a8.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/b1dfea10dd73.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/c07b88838d26.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/ca0a6149b070.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/cdc2683309a3.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/e0f8bbda3052.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/e42547405b98.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/f2e73601d105.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/f36e215fb3c6.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-ref-as-operand/fe04ebc07cb8.js` | `tests/lib/rules/no-ref-as-operand.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-000.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-001.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-002.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-003.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-004.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-005.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-006.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-008.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-009.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-010.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-011.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/11` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-012.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/12` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-013.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/13` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-014.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/14` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-015.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/15` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-016.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/16` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-017.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/17` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-018.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-021.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/21` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/valid-022.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `valid/22` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-000.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-001.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-002.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-003.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-004.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-005.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-006.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-007.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-008.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-010.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-011.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/11` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-012.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/12` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-013.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/13` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-014.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/14` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-015.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/15` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-016.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/16` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-017.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/17` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-018.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-019.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/19` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-020.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/20` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-022.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/22` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/invalid-024.vue` | `tests/lib/rules/no-mutating-props.test.ts` case `invalid/24` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/3ea88b6a3c85.vue` | `tests/lib/rules/no-mutating-props.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/a63ac1c32773.vue` | `tests/lib/rules/no-mutating-props.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/c69eb2fae189.vue` | `tests/lib/rules/no-mutating-props.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/ce690f09e944.vue` | `tests/lib/rules/no-mutating-props.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-mutating-props/dabf084bf553.vue` | `tests/lib/rules/no-mutating-props.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-000.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/0` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-001.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/1` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-002.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/2` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-003.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/3` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-004.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/4` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-005.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/5` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-006.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/6` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-007.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/7` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-008.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/8` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-009.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/9` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-010.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/10` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-011.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/11` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-012.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/12` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-013.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/13` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-014.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/14` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-015.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/15` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-016.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/16` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-017.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/17` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-018.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-019.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/19` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-020.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/20` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-021.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/21` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-022.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/22` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/valid-023.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `valid/23` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-000.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/0` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-001.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/1` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-002.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/2` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-003.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/3` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-005.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/5` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-006.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/6` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-007.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/7` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-008.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/8` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-009.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/9` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-010.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/10` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-013.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/13` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-014.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/14` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-015.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/15` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-016.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/16` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-017.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/17` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-018.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-019.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/19` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-020.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/20` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-021.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/21` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/invalid-022.vue` | `tests/lib/rules/no-dupe-keys.test.ts` case `invalid/22` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/080c1c26b9ea.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/092ad6ea02cc.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/0eefb298181c.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/2f55f7f59a18.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/3421ea4ca393.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/374c4af13156.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/403463dedf1b.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/468b3fc017ea.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/52cabafb8064.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/7b0a78abe3d0.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/8d7d11f08678.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/96f93fee2329.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/a636678cf478.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/a6d3c173827b.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/ac7193da1a29.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/ad252c3dcfb1.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/b2a9f6e3cf3b.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/b8a946ee8ee2.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/b8d2a2b3b0fa.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/b8f0c87726d2.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/bb5f2caea9f6.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/bd52102d9696.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/be28e2dc4b3a.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/c360658816fb.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/c6d2ec317b27.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/ceff5a1e3140.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/cfe70a776d7a.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/d506b625657b.vue` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/e19e050c5536.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/edc4e78e5b54.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/efa18d611e3c.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/f01ae360238b.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/f61651c5c9c0.vue` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-dupe-keys/fe62cb03428a.js` | `tests/lib/rules/no-dupe-keys.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/valid-000.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `valid/0` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/valid-001.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `valid/1` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/valid-002.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `valid/2` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/valid-004.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `valid/4` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/valid-006.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `valid/6` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-000.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/0` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-001.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/1` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-002.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/2` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-003.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/3` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-004.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/4` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-006.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/6` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-007.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/7` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-008.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/8` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-010.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/10` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-011.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/11` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-012.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/12` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-029.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/29` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-030.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/30` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-031.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/31` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-032.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/32` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-033.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/33` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-034.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/34` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-035.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/35` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/invalid-036.vue` | `tests/lib/rules/return-in-computed-property.test.ts` case `invalid/36` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/008d78807bf9.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/05e050c8c755.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/0cf508c1ab3b.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/13a579ca116a.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/147b98ec250f.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/14e7201ff23b.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/1b562501e9b7.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/20d71f3a5dc1.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/237840a438c7.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/2e6d0f1d8cc3.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/2f757ca213fa.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/311c459979fa.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/3179ea993fe4.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/32a74aee129d.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/3375c8e1e0f4.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/38fd8b49d244.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/3b3ca3f0baa4.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/442d476974cb.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/466f5c6dcb73.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/4fd8adbf8778.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/52e61cc7436e.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/53e5bd7345f4.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/540bca8f06c1.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/55f6ffb7376e.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/5a020d9c33e2.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/5ee14b60cda3.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/61a64276afc7.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/63300019196a.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/6e040e23254b.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/782278031ca4.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/7adb71b279b2.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/7b581a3dcbba.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/7db209ab88cc.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/7f0d008c4379.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/8e05b1717141.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/8e685222abdb.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/8f87aa1f797f.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/9092db114323.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/92ba2d0d49ec.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/9591aa794a1c.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/9b98f6089ab2.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/9c603e6d1a8e.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/9ceb857735f6.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/9d54504b391c.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/9f8eec6a7817.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/a077a717c829.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/a788c8abf7e3.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/b27be2d5870c.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/b83472581842.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/bc2e8e9eee06.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/c1355282b49e.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/c2502c514f3a.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/c4abc28ae72f.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/c84163b9cf7b.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/c8f56b229fe8.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/cdec721ad5e1.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/ce0d2869dc5b.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/cf6b93c39c38.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/d0e07233a99b.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/e0366888834d.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/e131c09ed164.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/e1897f137e4c.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/e308c1c45fd0.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/e5f8e89dc3a7.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/e615cbbdd1fc.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/e67c2399e106.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/e949c7f10ff4.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/eb32ea7d3fd7.js` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/eff415963631.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/f01b54e6d4fd.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/f609722b35e8.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/f9bb20dfc025.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/return-in-computed-property/fe881fa7c8df.vue` | `tests/lib/rules/return-in-computed-property.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/valid-000.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `valid/0` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/valid-001.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `valid/1` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/valid-002.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `valid/2` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/valid-003.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `valid/3` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/valid-004.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `valid/4` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/valid-005.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `valid/5` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/valid-006.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `valid/6` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/valid-007.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `valid/7` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/valid-008.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `valid/8` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/valid-009.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `valid/9` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/valid-010.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `valid/10` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/valid-011.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `valid/11` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/valid-012.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `valid/12` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/valid-013.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `valid/13` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/valid-014.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `valid/14` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-000.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/0` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-001.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/1` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-002.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/2` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-003.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/3` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-004.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/4` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-005.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/5` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-006.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/6` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-007.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/7` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-008.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/8` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-009.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/9` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-010.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/10` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-011.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/11` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-012.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/12` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-013.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/13` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-014.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/14` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-015.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/15` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-016.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/16` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-017.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/17` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-018.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/18` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-019.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/19` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-020.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/20` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-021.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/21` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-022.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/22` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-023.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/23` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-024.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/24` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-025.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/25` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-026.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/26` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-027.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/27` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-028.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/28` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-029.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/29` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-030.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/30` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-031.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/31` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-032.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/32` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-033.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/33` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-036.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/36` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-037.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/37` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-038.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/38` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/invalid-039.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` case `invalid/39` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/06fb9f767c5b.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/0e6cd7cd4f40.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/130e9c02bff8.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/1348c465bb21.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/17d7c484af31.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/18a14a60007e.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/1b8b991c2243.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/218ddd1c3c77.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/241231b4177c.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/2959e4a65278.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/2dda2cf75612.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/32791f7a2b54.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/34eedb0ba56c.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/34f2adda15b9.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/3db5388e4d14.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/5291bc0a4210.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/5322bf7f221a.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/65410522f851.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/6e62754c384c.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/6ea68b8dcaef.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/700460c3d6c9.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/80844f915d89.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/839cef6373ce.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/867b04f57001.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/8f9a555e94db.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/90ceb1cb4fb6.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/91a4f1d6298c.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/93ffdea2cb35.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/94686905d8b9.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/9725500dd959.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/985fb36691a4.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/9f705dede0b0.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/a1bc0c0556c2.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/a1c81da6f00b.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/a30f702ae4a2.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/ad11aab4f6ab.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/bc5fad326393.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/bef308cd289e.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/c635da22e293.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/c6ade39f5980.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/d3f624facdf8.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/d462dc89610f.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/db4e5a1387e3.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/e569c5380b7d.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/e5d3ff503a1b.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/ea151f31a175.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/eb2b7d962fa1.vue` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/ef00a66d826d.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/ef1daf89bab0.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/f0988c934445.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/f1da8639682e.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-async-in-computed-properties/f8c4275ad242.js` | `tests/lib/rules/no-async-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/valid-000.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `valid/0` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/valid-001.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `valid/1` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/valid-002.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `valid/2` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/valid-003.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `valid/3` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/valid-004.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `valid/4` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/valid-005.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `valid/5` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/valid-006.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `valid/6` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/valid-007.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `valid/7` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/valid-008.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `valid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/invalid-000.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `invalid/0` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/invalid-001.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `invalid/1` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/invalid-002.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `invalid/2` | derived: wrapped in <script lang="ts"> |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/invalid-003.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `invalid/3` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/invalid-004.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `invalid/4` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/invalid-005.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `invalid/5` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/invalid-006.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `invalid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/invalid-007.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `invalid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/invalid-008.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `invalid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/invalid-009.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `invalid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/invalid-010.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `invalid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/invalid-011.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `invalid/11` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/invalid-012.vue` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` case `invalid/12` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/0001215acb00.js` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/06d570f107cd.js` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/0a825c035d76.js` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/1cdc0445c80b.js` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/1faa4981d6e0.js` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/2296ded499dc.js` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/741b6c6adc4a.js` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/9038b33bab00.js` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/97a593f8ee17.js` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/99cad2c9c6ff.js` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/bf2b806c5216.js` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/bfd6b20d9925.js` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/cd5a83991ef1.ts` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-side-effects-in-computed-properties/ef8c20f3c7e8.js` | `tests/lib/rules/no-side-effects-in-computed-properties.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-export-in-script-setup/valid-000.vue` | `tests/lib/rules/no-export-in-script-setup.test.ts` case `valid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-export-in-script-setup/valid-001.vue` | `tests/lib/rules/no-export-in-script-setup.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-export-in-script-setup/valid-002.vue` | `tests/lib/rules/no-export-in-script-setup.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-export-in-script-setup/invalid-000.vue` | `tests/lib/rules/no-export-in-script-setup.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-export-in-script-setup/invalid-001.vue` | `tests/lib/rules/no-export-in-script-setup.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-export-in-script-setup/invalid-002.vue` | `tests/lib/rules/no-export-in-script-setup.test.ts` case `invalid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-000.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-001.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-002.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-003.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-004.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-005.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-006.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-007.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-008.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-009.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-010.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-011.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/11` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-012.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/12` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-013.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/13` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-014.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/14` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-015.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/15` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-016.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/16` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-017.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/17` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/valid-018.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `valid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-000.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-001.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-002.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-003.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-004.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-005.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/5` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-006.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-007.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-008.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-009.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-010.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-011.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/11` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-012.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/12` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-013.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/13` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-014.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/14` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-015.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/15` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-016.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/16` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-017.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/17` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-018.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-019.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/19` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-020.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/20` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-021.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/21` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-022.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/22` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-023.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/23` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/invalid-024.vue` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` case `invalid/24` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-setup-props-reactivity-loss/fe3b16368f1b.js` | `tests/lib/rules/no-setup-props-reactivity-loss.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/no-watch-after-await/valid-000.vue` | `tests/lib/rules/no-watch-after-await.test.ts` case `valid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-watch-after-await/valid-001.vue` | `tests/lib/rules/no-watch-after-await.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-watch-after-await/valid-002.vue` | `tests/lib/rules/no-watch-after-await.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-watch-after-await/valid-003.vue` | `tests/lib/rules/no-watch-after-await.test.ts` case `valid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-watch-after-await/valid-004.vue` | `tests/lib/rules/no-watch-after-await.test.ts` case `valid/4` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-watch-after-await/valid-005.vue` | `tests/lib/rules/no-watch-after-await.test.ts` case `valid/5` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/no-watch-after-await/valid-006.vue` | `tests/lib/rules/no-watch-after-await.test.ts` case `valid/6` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-watch-after-await/valid-007.vue` | `tests/lib/rules/no-watch-after-await.test.ts` case `valid/7` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-watch-after-await/valid-008.vue` | `tests/lib/rules/no-watch-after-await.test.ts` case `valid/8` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-watch-after-await/valid-009.vue` | `tests/lib/rules/no-watch-after-await.test.ts` case `valid/9` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-watch-after-await/valid-010.vue` | `tests/lib/rules/no-watch-after-await.test.ts` case `valid/10` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-watch-after-await/invalid-000.vue` | `tests/lib/rules/no-watch-after-await.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-watch-after-await/invalid-001.vue` | `tests/lib/rules/no-watch-after-await.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-watch-after-await/invalid-002.vue` | `tests/lib/rules/no-watch-after-await.test.ts` case `invalid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-watch-after-await/e32d3b09b69d.js` | `tests/lib/rules/no-watch-after-await.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-000.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/0` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-001.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/1` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-002.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/2` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-003.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/3` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-004.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/4` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-005.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/5` | derived: wrapped in <script lang="ts"> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-006.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/6` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-007.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/7` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-008.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/8` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-009.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/9` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-010.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/10` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-011.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/11` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-012.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/12` | derived: wrapped in <script lang="ts"> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-013.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/13` | derived: wrapped in <script lang="ts"> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-014.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/14` | derived: wrapped in <script lang="ts"> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-015.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/15` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-017.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/17` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-018.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/18` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-020.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/20` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-021.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/21` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-022.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/22` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-023.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/23` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-024.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/24` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-025.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/25` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/valid-026.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `valid/26` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-000.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/0` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-001.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/1` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-002.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/2` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-003.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/3` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-004.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/4` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-005.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/5` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-006.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/6` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-007.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/7` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-008.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/8` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-009.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/9` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-010.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/10` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-011.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/11` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-012.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/12` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-013.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/13` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-014.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/14` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-015.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/15` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-016.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/16` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-017.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/17` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-018.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/18` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-019.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/19` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-020.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/20` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-021.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/21` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-022.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/22` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-023.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/23` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-024.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/24` | derived: wrapped in <script lang="ts"> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-025.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/25` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-026.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/26` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-027.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/27` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-028.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/28` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-029.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/29` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-030.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/30` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-031.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/31` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-032.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/32` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-033.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/33` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-034.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/34` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-035.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/35` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-036.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/36` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-037.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/37` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-038.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/38` | derived: wrapped in <script> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-039.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/39` | derived: wrapped in <script lang="ts"> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-040.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/40` | derived: wrapped in <script lang="ts"> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-041.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/41` | derived: wrapped in <script lang="ts"> |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-042.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/42` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-045.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/45` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-046.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/46` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-047.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/47` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-048.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/48` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-049.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/49` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-050.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/50` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-052.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/52` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-053.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/53` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-054.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/54` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-055.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/55` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/invalid-056.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` case `invalid/56` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/001757d5ff3c.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/0854539d2a00.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/0cc637984de6.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/0f818368dfb6.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/11739bd641e0.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/12b6e3a38580.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/167067a228a3.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/193b12167a03.ts` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/1b69fa08e972.tsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/2842b95a0bfa.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/2c19c7444424.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/2e1e587ab8e1.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/308a33e31e0a.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/3379af2eea50.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/3baad3af3e37.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/3d5cd65ae75e.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/3ddd71522d85.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/419e863b9501.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/4237414410ca.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/42e06e3a9e47.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/44fba13ac893.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/47d64eafb58f.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/47dafad87185.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/4be3a6702c26.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/4ed4a436b32a.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/59aee9b4b7d9.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/6021b65d1b31.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/638440bb1060.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/67714fd76512.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/67a1ae766704.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/67b8257edf67.ts` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/6a88b0016ffc.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/6bcc1c0423f6.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/6d93a84c3fe6.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/6e0ba50856eb.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/7079d6d5b63d.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/75d76e019422.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/7675c485496f.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/83e02a704178.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/84091795ae03.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/87a663afd6c1.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/8a735b1ee18c.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/8b19983b909e.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/8b62fe0daa1a.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/93f9610a09d0.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/a6416259a040.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/a8546c5a8bf2.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/aaf4f4daf09d.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/ab563145d8d3.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/abb97da41bbb.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/b43935df3676.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/b790e0be6aa0.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/bac869dc39cd.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/bcda8d7f5327.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/be36a38cd629.ts` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/c59a51c3c5b8.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/c6f72d831cbf.ts` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/c8bccd9d7fb6.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/cbb75a35427d.ts` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/cff0a2474030.vue` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/d05eff743688.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/e1c6b696e624.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/e57c983b250d.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/e8357a7b8e19.ts` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/fc4f6cc99f71.ts` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |
| `upstream/eslint-plugin-vue/fixtures/require-valid-default-prop/fdff4bd671b7.jsx` | `tests/lib/rules/require-valid-default-prop.test.ts` | derived |

> `recording.json` is a port readiness recording, and no file of this pack, so it is recorded here rather than in the
> table above: the tool-pack's `recording.lock` names it and a GitHub release holds the bytes (docs/port-readiness.md).
> It carries excerpts of this upstream's tests (`tests/lib/rules/**/*.test.ts`: the `code` of each RuleTester case),
> redistributed under the licence above, together with Rerail's Apache-2.0; `LICENSE` beside this file is this
> upstream's own, at the pinned commit.
