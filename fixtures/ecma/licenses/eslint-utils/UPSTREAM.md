# @eslint-community/eslint-utils 4.10.1

Upstream: [@eslint-community/eslint-utils 4.10.1](https://github.com/eslint-community/eslint-utils/tree/v4.10.1). License: MIT (Copyright (c) 2018 Toru Nagashima); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at that version.

The files listed below contain material adapted from eslint-utils and remain under its license. Every other file in this
pack is Rerail's, under Rerail's MIT license (`rerail/LICENSE`): the `generate_*` scripts, the `*.expected.json`
outputs recorded by running upstream tools, and the fixtures and cases written for Rerail.

Paths are relative to `rerail-fixtures/ecma/`.

| File | Upstream path | Copy |
|---|---|---|
| `scope/generate_typescript.mjs` | `index.mjs` | derived: adapted `isEffectivelyConst`, `canBeConsideredConst`, and `hasMutationInProperty` helpers. |
