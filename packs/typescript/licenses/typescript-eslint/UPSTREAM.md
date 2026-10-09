# typescript-eslint 8.70.1

Upstream: [typescript-eslint 8.70.1](https://github.com/typescript-eslint/typescript-eslint/tree/v8.70.1) (commit `23d38ceeb8fb1237f55cb62fc2b54419e1fa1ed7`). License: MIT (Copyright (c) 2019 typescript-eslint and other contributors); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at that tag.

The files listed below contain material copied from typescript-eslint and remain under its license. Every other file in this
pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of ESLint's
and typescript-eslint's rules (and, in `candidates/rules/vitest-*.json`, of three @vitest/eslint-plugin 1.6.27 rules,
which are MIT too), under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The rest
is under the Apache License 2.0 (the pack's `LICENSE`): the recorder `upstream/generate.mjs`, and the `*.expected.json`
recordings it writes by running ESLint.

Each copied file is the `code` of one of the rule's own `valid` or `invalid` RuleTester cases, verbatim, written
by `upstream/generate.mjs`: cases with rule options, a custom parser or type information are left out, and identical
code is written once.

Paths are relative to the pack directory, `packs/typescript/`.

| File | Upstream path | Copy |
|---|---|---|
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/001.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/002.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/003.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/004.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/005.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/006.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/007.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/008.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/009.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/010.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/011.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/012.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/013.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/014.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/015.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/016.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/017.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/018.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/019.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/020.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/021.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/022.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/023.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/024.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/025.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/026.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/027.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/028.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/029.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/030.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/031.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/032.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/033.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/034.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/035.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/036.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/037.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/038.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/039.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-duplicate-enum-values/040.ts` | `packages/eslint-plugin/tests/rules/no-duplicate-enum-values.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-namespace/001.ts` | `packages/eslint-plugin/tests/rules/no-namespace.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-namespace/002.ts` | `packages/eslint-plugin/tests/rules/no-namespace.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-namespace/003.ts` | `packages/eslint-plugin/tests/rules/no-namespace.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-namespace/004.ts` | `packages/eslint-plugin/tests/rules/no-namespace.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-namespace/005.ts` | `packages/eslint-plugin/tests/rules/no-namespace.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-namespace/006.ts` | `packages/eslint-plugin/tests/rules/no-namespace.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-namespace/007.ts` | `packages/eslint-plugin/tests/rules/no-namespace.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-unsafe-function-type/001.ts` | `packages/eslint-plugin/tests/rules/no-unsafe-function-type.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-unsafe-function-type/002.ts` | `packages/eslint-plugin/tests/rules/no-unsafe-function-type.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-unsafe-function-type/003.ts` | `packages/eslint-plugin/tests/rules/no-unsafe-function-type.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-unsafe-function-type/004.ts` | `packages/eslint-plugin/tests/rules/no-unsafe-function-type.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-unsafe-function-type/005.ts` | `packages/eslint-plugin/tests/rules/no-unsafe-function-type.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-unsafe-function-type/006.ts` | `packages/eslint-plugin/tests/rules/no-unsafe-function-type.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-unsafe-function-type/007.ts` | `packages/eslint-plugin/tests/rules/no-unsafe-function-type.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-unsafe-function-type/008.ts` | `packages/eslint-plugin/tests/rules/no-unsafe-function-type.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/001.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/002.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/003.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/004.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/005.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/006.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/007.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/008.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/009.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/010.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/011.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/012.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/013.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/014.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/015.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/016.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/017.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/018.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/019.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/020.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/021.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/022.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/023.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/024.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/025.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/026.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/027.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/028.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/029.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/030.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/031.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/032.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/033.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/034.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/035.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/036.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/037.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/038.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/039.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/040.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/041.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/042.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/043.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/044.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/045.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/046.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/047.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/048.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/049.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/050.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/051.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/052.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/053.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/054.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/055.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/056.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/057.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/no-wrapper-object-types/058.ts` | `packages/eslint-plugin/tests/rules/no-wrapper-object-types.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/001.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/002.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/003.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/004.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/005.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/006.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/007.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/008.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/009.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/010.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/011.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/012.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/013.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/014.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/015.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/016.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/017.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/018.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/019.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/020.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/021.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/022.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/023.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/024.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/025.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/026.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/027.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/028.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/029.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/030.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/031.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/032.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/033.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/034.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/035.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/036.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/037.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/038.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/039.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/040.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/041.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/042.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/043.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/044.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/045.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
| `upstream/typescript-eslint/fixtures/prefer-as-const/046.ts` | `packages/eslint-plugin/tests/rules/prefer-as-const.test.ts` | derived |
