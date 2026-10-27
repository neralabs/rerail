# eslint-plugin-vue 10.11.1

Upstream: [vuejs/eslint-plugin-vue](https://github.com/vuejs/eslint-plugin-vue/tree/ec9c53a6fbed637d7b0fbcd6893b0206ae32839f) at tag `v10.11.1` (commit `ec9c53a6fbed637d7b0fbcd6893b0206ae32839f`). License: MIT (Copyright (c) 2017 Toru Nagashima); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at that tag.

The files listed below are the code of RuleTester cases of the rules the Vue pack ports (`../../conformance.json`), and remain under eslint-plugin-vue's license. Each is one case's `code`, copied verbatim, or wrapped in a `<script>` element when the case is a script (a non-`.vue` filename, a parser other than vue-eslint-parser, or code with no SFC block and no filename), so that it is a `.vue` file. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of eslint-plugin-vue's rules, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. Every other file under `upstream/` is Rerail's, under the Apache License 2.0 (this repository's `LICENSE`): `read_tests.mjs`, `generate.mjs`, `authored.mjs`, `cases.json`, the fixtures `rerail-*.vue` (written by Rerail; `upstream/eslint-plugin-vue/UPSTREAM.md` lists them) and the expected findings they write by running ESLint 10.11.0 with eslint-plugin-vue 10.11.1.

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
| `upstream/eslint-plugin-vue/fixtures/no-v-html/valid-001.vue` | `tests/lib/rules/no-v-html.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-html/valid-002.vue` | `tests/lib/rules/no-v-html.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-html/valid-003.vue` | `tests/lib/rules/no-v-html.test.ts` case `valid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-html/invalid-000.vue` | `tests/lib/rules/no-v-html.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-html/invalid-001.vue` | `tests/lib/rules/no-v-html.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/no-v-html/invalid-002.vue` | `tests/lib/rules/no-v-html.test.ts` case `invalid/2` | verbatim |
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
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/valid-000.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` case `valid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/valid-001.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` case `valid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/valid-002.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` case `valid/2` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/valid-003.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` case `valid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/invalid-000.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` case `invalid/0` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/invalid-001.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` case `invalid/1` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/invalid-003.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` case `invalid/3` | verbatim |
| `upstream/eslint-plugin-vue/fixtures/v-on-event-hyphenation/invalid-005.vue` | `tests/lib/rules/v-on-event-hyphenation.test.ts` case `invalid/5` | verbatim |
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
