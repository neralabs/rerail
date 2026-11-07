# eslint-plugin-n 18.4.1

Upstream: [eslint-community/eslint-plugin-n](https://github.com/eslint-community/eslint-plugin-n/tree/f1d47aff402b9e009578f4b3155b84f7d8beef9b) at tag `v18.4.1` (commit `f1d47aff402b9e009578f4b3155b84f7d8beef9b`). License: MIT (Copyright (c) 2015 Toru Nagashima); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at that tag.

The files listed below are the upstream rule tests of the rules the Node pack records (`conformance.json`, `upstream/eslint-plugin-n/generate.mjs`), the upstream test data they read, and one file per test case holding that case's `code` verbatim; they remain under eslint-plugin-n's license. One listed file is Rerail's and says so in its row: `upstream/eslint-plugin-n/fixtures/package.json` (the fixtures' source type and engines), listed because it sits among the copied fixtures. Every other file in this pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of eslint-plugin-n's rules, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the recorder `upstream/eslint-plugin-n/generate.mjs`, `upstream/eslint-plugin-n/stub-test-helpers.js` (stands in for upstream's `tests/test-helpers.js` and only captures cases), and the `*.expected.json` recordings written by running eslint-plugin-n.

Paths are relative to the pack directory, `packs/node/`.

|File|Upstream path|Copy|
|---|---|---|
|`upstream/eslint-plugin-n/tests/lib/rules/callback-return.js`|`tests/lib/rules/callback-return.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/exports-style.js`|`tests/lib/rules/exports-style.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/global-require.js`|`tests/lib/rules/global-require.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/handle-callback-err.js`|`tests/lib/rules/handle-callback-err.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/hashbang.js`|`tests/lib/rules/hashbang.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/no-callback-literal.js`|`tests/lib/rules/no-callback-literal.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/no-deprecated-api.js`|`tests/lib/rules/no-deprecated-api.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/no-exports-assign.js`|`tests/lib/rules/no-exports-assign.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/no-mixed-requires.js`|`tests/lib/rules/no-mixed-requires.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/no-new-require.js`|`tests/lib/rules/no-new-require.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/no-path-concat.js`|`tests/lib/rules/no-path-concat.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/no-process-env.js`|`tests/lib/rules/no-process-env.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/no-process-exit.js`|`tests/lib/rules/no-process-exit.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/no-sync.js`|`tests/lib/rules/no-sync.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/prefer-global/buffer.js`|`tests/lib/rules/prefer-global/buffer.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/prefer-global/console.js`|`tests/lib/rules/prefer-global/console.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/prefer-global/process.js`|`tests/lib/rules/prefer-global/process.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/prefer-global/text-decoder.js`|`tests/lib/rules/prefer-global/text-decoder.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/prefer-global/text-encoder.js`|`tests/lib/rules/prefer-global/text-encoder.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/prefer-global/url.js`|`tests/lib/rules/prefer-global/url.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/prefer-global/url-search-params.js`|`tests/lib/rules/prefer-global/url-search-params.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/prefer-node-protocol.js`|`tests/lib/rules/prefer-node-protocol.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/prefer-promises/dns.js`|`tests/lib/rules/prefer-promises/dns.js`|verbatim|
|`upstream/eslint-plugin-n/tests/lib/rules/prefer-promises/fs.js`|`tests/lib/rules/prefer-promises/fs.js`|verbatim|
|`upstream/eslint-plugin-n/fixtures/package.json`|none|Rerail's own, not copied: the fixtures' source type and engines (`fixturePackageJson`), under the pack's `LICENSE`|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-001/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-002/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-003/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-004/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-005/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-006/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-007/shebang/no-bin-field/package.json`|`tests/fixtures/shebang/no-bin-field/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-008/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-009/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-010/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-011/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-012/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-013/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-014/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-015/shebang/no-bin-field/package.json`|`tests/fixtures/shebang/no-bin-field/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-016/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-017/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-018/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-019/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-020/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-021/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-022/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-023/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-024/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-025/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-026/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-027/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-028/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-029/shebang/unpublished/package.json`|`tests/fixtures/shebang/unpublished/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-030/shebang/unpublished/package.json`|`tests/fixtures/shebang/unpublished/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-031/shebang/unpublished/package.json`|`tests/fixtures/shebang/unpublished/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-032/shebang/unpublished/package.json`|`tests/fixtures/shebang/unpublished/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-033/shebang/unpublished/package.json`|`tests/fixtures/shebang/unpublished/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-034/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-035/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-001/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-002/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-003/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-004/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-005/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-006/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-007/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-008/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-009/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-010/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-011/shebang/no-bin-field/package.json`|`tests/fixtures/shebang/no-bin-field/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-014/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-015/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-016/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-017/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-018/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-019/shebang/no-bin-field/package.json`|`tests/fixtures/shebang/no-bin-field/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-020/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-021/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-022/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-023/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-024/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-025/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-026/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-027/shebang/string-bin/package.json`|`tests/fixtures/shebang/string-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-028/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-029/shebang/unpublished/package.json`|`tests/fixtures/shebang/unpublished/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-030/shebang/unpublished/package.json`|`tests/fixtures/shebang/unpublished/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-031/shebang/unpublished/package.json`|`tests/fixtures/shebang/unpublished/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-032/shebang/unpublished/package.json`|`tests/fixtures/shebang/unpublished/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-033/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-034/shebang/object-bin/package.json`|`tests/fixtures/shebang/object-bin/package.json`|verbatim (test data a case's `filename` reads)|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-001.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-002.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-003.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-004.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-005.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-006.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-007.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[6]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-008.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[7]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-009.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[8]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-010.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[9]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-011.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[10]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-012.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[11]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-013.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[12]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-014.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[13]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-015.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[14]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-016.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[15]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-017.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[16]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-018.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[17]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-019.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[18]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-020.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[19]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-021.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[20]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-022.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[21]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-023.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[22]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-024.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[23]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-025.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[24]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-026.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[25]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-027.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[26]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-028.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[27]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-029.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[28]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-030.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[29]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-031.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[30]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-032.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[31]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-033.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[32]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-034.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[33]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-035.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[34]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-036.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[35]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-037.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[36]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-038.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[37]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-039.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[38]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-040.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[39]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-041.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[40]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-042.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[41]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-043.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[42]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-044.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[43]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-045.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[44]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-046.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[45]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-047.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[46]|
|`upstream/eslint-plugin-n/fixtures/callback-return/valid-048.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js valid[47]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-001.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-002.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-003.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-004.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-005.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-006.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-007.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-008.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-009.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-010.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[9]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-011.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[10]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-012.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[11]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-013.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[12]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-014.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[13]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-015.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[14]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-016.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[15]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-017.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[16]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-018.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[17]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-019.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[18]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-020.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[19]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-021.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[20]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-022.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[21]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-023.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[22]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-024.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[23]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-025.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[24]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-026.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[25]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-027.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[26]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-028.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[27]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-029.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[28]|
|`upstream/eslint-plugin-n/fixtures/callback-return/invalid-030.js`|`tests/lib/rules/callback-return.js`|extracted from tests/lib/rules/callback-return.js invalid[29]|
|`upstream/eslint-plugin-n/fixtures/exports-style/valid-001.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/exports-style/valid-002.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/exports-style/valid-003.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/exports-style/valid-004.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/exports-style/valid-005.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/exports-style/valid-006.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/exports-style/valid-007.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js valid[6]|
|`upstream/eslint-plugin-n/fixtures/exports-style/valid-008.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js valid[7]|
|`upstream/eslint-plugin-n/fixtures/exports-style/valid-009.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js valid[8]|
|`upstream/eslint-plugin-n/fixtures/exports-style/valid-010.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js valid[9]|
|`upstream/eslint-plugin-n/fixtures/exports-style/valid-011.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js valid[10]|
|`upstream/eslint-plugin-n/fixtures/exports-style/valid-012.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js valid[11]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-001.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-002.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-003.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-004.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-005.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-006.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-007.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-008.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-009.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-010.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[9]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-011.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[10]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-012.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[11]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-013.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[12]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-014.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[13]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-015.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[14]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-016.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[15]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-017.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[16]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-018.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[17]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-019.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[18]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-020.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[19]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-021.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[20]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-022.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[21]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-023.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[22]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-024.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[23]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-025.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[24]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-026.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[25]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-027.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[26]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-028.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[27]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-029.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[28]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-030.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[29]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-031.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[30]|
|`upstream/eslint-plugin-n/fixtures/exports-style/invalid-032.js`|`tests/lib/rules/exports-style.js`|extracted from tests/lib/rules/exports-style.js invalid[31]|
|`upstream/eslint-plugin-n/fixtures/global-require/valid-001.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/global-require/valid-002.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/global-require/valid-003.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/global-require/valid-004.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/global-require/valid-005.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/global-require/valid-006.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/global-require/valid-007.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js valid[6]|
|`upstream/eslint-plugin-n/fixtures/global-require/valid-008.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js valid[7]|
|`upstream/eslint-plugin-n/fixtures/global-require/valid-009.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js valid[8]|
|`upstream/eslint-plugin-n/fixtures/global-require/valid-010.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js valid[9]|
|`upstream/eslint-plugin-n/fixtures/global-require/valid-011.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js valid[10]|
|`upstream/eslint-plugin-n/fixtures/global-require/valid-012.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js valid[11]|
|`upstream/eslint-plugin-n/fixtures/global-require/valid-013.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js valid[12]|
|`upstream/eslint-plugin-n/fixtures/global-require/invalid-001.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/global-require/invalid-002.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/global-require/invalid-003.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/global-require/invalid-004.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/global-require/invalid-005.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/global-require/invalid-006.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/global-require/invalid-007.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/global-require/invalid-008.js`|`tests/lib/rules/global-require.js`|extracted from tests/lib/rules/global-require.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-001.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-002.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-003.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-004.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-005.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-006.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-007.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[6]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-008.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[7]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-009.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[8]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-010.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[9]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-011.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[10]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-012.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[11]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-013.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[12]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-014.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[13]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-015.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[14]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-016.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[15]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-017.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[16]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-018.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[17]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-019.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[18]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-020.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[19]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-021.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[20]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-022.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[21]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-023.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[22]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-024.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[23]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-025.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[24]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-026.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[25]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/valid-027.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js valid[26]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-001.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-002.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-003.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-004.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-005.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-006.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-007.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-008.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-009.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-010.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[9]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-011.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[10]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-012.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[11]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-013.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[12]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-014.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[13]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-015.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[14]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-016.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[15]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-017.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[16]|
|`upstream/eslint-plugin-n/fixtures/handle-callback-err/invalid-018.js`|`tests/lib/rules/handle-callback-err.js`|extracted from tests/lib/rules/handle-callback-err.js invalid[17]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-001/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-002/shebang/string-bin/lib/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-003/shebang/object-bin/bin/a.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-004/shebang/object-bin/bin/b.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-005/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-006/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-007/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[6]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-008/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[7]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-009/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[8]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-010/shebang/object-bin/bin/c.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[9]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-011/shebang/no-bin-field/lib/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[10]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-012.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[11]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-013.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[12]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-014/shebang/string-bin/src/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[13]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-015/shebang/string-bin/src/lib/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[14]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-016/shebang/object-bin/src/bin/a.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[15]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-017/shebang/object-bin/src/bin/b.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[16]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-018/shebang/object-bin/src/bin/c.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[17]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-019/shebang/no-bin-field/src/lib/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[18]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-020/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[19]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-021/shebang/string-bin/lib/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[20]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-022/shebang/string-bin/lib/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[21]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-023/shebang/string-bin/lib/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[22]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-024/shebang/string-bin/lib/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[23]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-025/shebang/string-bin/lib/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[24]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-026/shebang/string-bin/lib/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[25]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-027/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[26]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-028/shebang/object-bin/bin/index.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[27]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-029/shebang/unpublished/published.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[28]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-030/shebang/unpublished/unpublished.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[29]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-031/shebang/unpublished/unpublished.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[30]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-032/shebang/unpublished/something.test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[31]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-033/shebang/object-bin/bin/t.ts`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[32]|
|`upstream/eslint-plugin-n/fixtures/hashbang/valid-034/shebang/object-bin/bin/a.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js valid[33]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-001/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-002/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-003/shebang/string-bin/lib/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-004/shebang/object-bin/bin/a.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-005/shebang/object-bin/bin/b.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-006/shebang/object-bin/bin/c.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-007/shebang/no-bin-field/lib/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-008/shebang/string-bin/src/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-009/shebang/string-bin/src/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-010/shebang/string-bin/src/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[9]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-011/shebang/string-bin/src/lib/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[10]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-012/shebang/object-bin/src/bin/a.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[11]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-013/shebang/object-bin/src/bin/b.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[12]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-014/shebang/object-bin/src/bin/c.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[13]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-015/shebang/no-bin-field/src/lib/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[14]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-016/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[15]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-017/shebang/string-bin/lib/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[16]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-018/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[17]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-019/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[18]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-020/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[19]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-021/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[20]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-022/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[21]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-023/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[22]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-024/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[23]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-025/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[24]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-026/shebang/string-bin/bin/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[25]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-027/shebang/string-bin/lib/test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[26]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-028/shebang/object-bin/bin/index.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[27]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-029/shebang/unpublished/unpublished.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[28]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-030/shebang/unpublished/published.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[29]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-031/shebang/unpublished/published.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[30]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-032/shebang/unpublished/something.test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[31]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-033/shebang/unpublished/not-a-test.js`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[32]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-034/shebang/object-bin/bin/t.ts`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[33]|
|`upstream/eslint-plugin-n/fixtures/hashbang/invalid-035/shebang/object-bin/bin/t.ts`|`tests/lib/rules/hashbang.js`|extracted from tests/lib/rules/hashbang.js invalid[34]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-001.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-002.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-003.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-004.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-005.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-006.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-007.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[6]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-008.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[7]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-009.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[8]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-010.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[9]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-011.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[10]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-012.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[11]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-013.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[12]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-014.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[13]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-015.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[14]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-016.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[15]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-017.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[16]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-018.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[17]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-019.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[18]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-020.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[19]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-021.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[20]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-022.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[21]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-023.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[22]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-024.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[23]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-025.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[24]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/valid-026.ts`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js valid[25]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/invalid-001.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/invalid-002.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/invalid-003.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/invalid-004.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/invalid-005.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/invalid-006.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/invalid-007.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/invalid-008.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/invalid-009.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/no-callback-literal/invalid-010.js`|`tests/lib/rules/no-callback-literal.js`|extracted from tests/lib/rules/no-callback-literal.js invalid[9]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-001.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-002.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-003.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-004.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-005.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-006.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-007.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[6]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-008.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[7]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-009.mjs`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[8]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-010.mjs`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[9]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-011.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[10]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-012.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[11]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-013.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[12]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-014.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[13]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-015.mjs`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[14]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-016.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[15]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-017.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[16]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-018.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[17]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-019.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[18]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-020.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[19]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-021.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[20]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-022.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[21]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-023.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[22]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-024.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[23]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-025.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[24]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-026.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[25]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-027.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[26]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/valid-028.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js valid[27]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-001.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-002.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-003.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-004.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-005.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-006.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-007.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-008.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-009.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-010.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[9]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-011.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[10]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-012.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[11]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-013.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[12]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-014.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[13]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-015.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[14]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-016.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[15]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-017.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[16]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-018.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[17]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-019.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[18]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-020.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[19]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-021.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[20]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-022.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[21]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-023.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[22]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-024.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[23]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-025.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[24]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-026.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[25]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-027.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[26]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-028.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[27]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-029.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[28]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-030.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[29]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-031.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[30]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-032.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[31]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-033.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[32]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-034.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[33]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-035.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[34]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-036.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[35]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-037.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[36]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-038.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[37]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-039.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[38]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-040.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[39]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-041.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[40]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-042.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[41]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-043.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[42]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-044.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[43]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-045.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[44]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-046.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[45]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-047.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[46]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-048.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[47]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-049.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[48]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-050.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[49]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-051.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[50]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-052.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[51]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-053.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[52]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-054.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[53]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-055.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[54]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-056.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[55]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-057.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[56]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-058.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[57]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-059.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[58]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-060.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[59]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-061.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[60]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-062.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[61]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-063.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[62]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-064.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[63]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-065.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[64]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-066.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[65]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-067.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[66]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-068.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[67]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-069.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[68]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-070.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[69]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-071.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[70]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-072.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[71]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-073.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[72]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-074.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[73]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-075.mjs`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[74]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-076.mjs`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[75]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-077.mjs`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[76]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-078.mjs`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[77]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-079.mjs`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[78]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-080.mjs`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[79]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-081.mjs`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[80]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-082.mjs`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[81]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-083.mjs`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[82]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-084.mjs`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[83]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-085.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[84]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-086.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[85]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-087.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[86]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-088.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[87]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-089.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[88]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-090.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[89]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-091.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[90]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-092.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[91]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-093.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[92]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-094.mjs`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[93]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-095.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[94]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-096.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[95]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-097.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[96]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-098.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[97]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-099.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[98]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-100.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[99]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-101.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[100]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-102.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[101]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-103.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[102]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-104.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[103]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-105.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[104]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-106.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[105]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-107.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[106]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-108.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[107]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-109.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[108]|
|`upstream/eslint-plugin-n/fixtures/no-deprecated-api/invalid-110.js`|`tests/lib/rules/no-deprecated-api.js`|extracted from tests/lib/rules/no-deprecated-api.js invalid[109]|
|`upstream/eslint-plugin-n/fixtures/no-exports-assign/valid-001.js`|`tests/lib/rules/no-exports-assign.js`|extracted from tests/lib/rules/no-exports-assign.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/no-exports-assign/valid-002.js`|`tests/lib/rules/no-exports-assign.js`|extracted from tests/lib/rules/no-exports-assign.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/no-exports-assign/valid-003.js`|`tests/lib/rules/no-exports-assign.js`|extracted from tests/lib/rules/no-exports-assign.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/no-exports-assign/valid-004.js`|`tests/lib/rules/no-exports-assign.js`|extracted from tests/lib/rules/no-exports-assign.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/no-exports-assign/valid-005.js`|`tests/lib/rules/no-exports-assign.js`|extracted from tests/lib/rules/no-exports-assign.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/no-exports-assign/invalid-001.js`|`tests/lib/rules/no-exports-assign.js`|extracted from tests/lib/rules/no-exports-assign.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/valid-001.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/valid-002.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/valid-003.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/valid-004.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/valid-005.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/valid-006.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/valid-007.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js valid[6]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/valid-008.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js valid[7]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/valid-009.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js valid[8]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/valid-010.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js valid[9]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/valid-011.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js valid[10]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/valid-012.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js valid[11]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/valid-013.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js valid[12]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/invalid-001.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/invalid-002.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/invalid-003.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/invalid-004.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/invalid-005.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/invalid-006.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/invalid-007.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/invalid-008.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/invalid-009.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/no-mixed-requires/invalid-010.js`|`tests/lib/rules/no-mixed-requires.js`|extracted from tests/lib/rules/no-mixed-requires.js invalid[9]|
|`upstream/eslint-plugin-n/fixtures/no-new-require/valid-001.js`|`tests/lib/rules/no-new-require.js`|extracted from tests/lib/rules/no-new-require.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/no-new-require/valid-002.js`|`tests/lib/rules/no-new-require.js`|extracted from tests/lib/rules/no-new-require.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/no-new-require/valid-003.js`|`tests/lib/rules/no-new-require.js`|extracted from tests/lib/rules/no-new-require.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/no-new-require/invalid-001.js`|`tests/lib/rules/no-new-require.js`|extracted from tests/lib/rules/no-new-require.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/no-new-require/invalid-002.js`|`tests/lib/rules/no-new-require.js`|extracted from tests/lib/rules/no-new-require.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/valid-001.mjs`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/valid-002.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/valid-003.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/valid-004.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/valid-005.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/valid-006.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/valid-007.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js valid[6]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/valid-008.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js valid[7]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/valid-009.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js valid[8]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/valid-010.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js valid[9]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/valid-011.mjs`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js valid[10]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/valid-012.mjs`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js valid[11]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/valid-013.mjs`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js valid[12]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-001.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-002.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-003.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-004.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-005.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-006.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-007.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-008.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-009.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-010.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[9]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-011.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[10]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-012.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[11]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-013.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[12]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-014.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[13]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-015.js`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[14]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-016.mjs`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[15]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-017.mjs`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[16]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-018.mjs`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[17]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-019.mjs`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[18]|
|`upstream/eslint-plugin-n/fixtures/no-path-concat/invalid-020.mjs`|`tests/lib/rules/no-path-concat.js`|extracted from tests/lib/rules/no-path-concat.js invalid[19]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/valid-001.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/valid-002.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/valid-003.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/valid-004.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/valid-005.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/valid-006.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/valid-007.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js valid[6]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/valid-008.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js valid[7]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/invalid-001.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/invalid-002.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/invalid-003.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/invalid-004.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/invalid-005.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/invalid-006.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/invalid-007.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/invalid-008.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/invalid-009.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/no-process-env/invalid-010.js`|`tests/lib/rules/no-process-env.js`|extracted from tests/lib/rules/no-process-env.js invalid[9]|
|`upstream/eslint-plugin-n/fixtures/no-process-exit/valid-001.js`|`tests/lib/rules/no-process-exit.js`|extracted from tests/lib/rules/no-process-exit.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/no-process-exit/valid-002.js`|`tests/lib/rules/no-process-exit.js`|extracted from tests/lib/rules/no-process-exit.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/no-process-exit/valid-003.js`|`tests/lib/rules/no-process-exit.js`|extracted from tests/lib/rules/no-process-exit.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/no-process-exit/invalid-001.js`|`tests/lib/rules/no-process-exit.js`|extracted from tests/lib/rules/no-process-exit.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/no-process-exit/invalid-002.js`|`tests/lib/rules/no-process-exit.js`|extracted from tests/lib/rules/no-process-exit.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/no-process-exit/invalid-003.js`|`tests/lib/rules/no-process-exit.js`|extracted from tests/lib/rules/no-process-exit.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/no-sync/valid-001.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 valid[0]|
|`upstream/eslint-plugin-n/fixtures/no-sync/valid-002.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 valid[1]|
|`upstream/eslint-plugin-n/fixtures/no-sync/valid-003.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 valid[2]|
|`upstream/eslint-plugin-n/fixtures/no-sync/valid-004.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 valid[3]|
|`upstream/eslint-plugin-n/fixtures/no-sync/valid-005.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 valid[4]|
|`upstream/eslint-plugin-n/fixtures/no-sync/valid-006.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 valid[5]|
|`upstream/eslint-plugin-n/fixtures/no-sync/valid-007.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 valid[6]|
|`upstream/eslint-plugin-n/fixtures/no-sync/valid-008.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 valid[7]|
|`upstream/eslint-plugin-n/fixtures/no-sync/valid-009.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 valid[8]|
|`upstream/eslint-plugin-n/fixtures/no-sync/invalid-001.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 invalid[0]|
|`upstream/eslint-plugin-n/fixtures/no-sync/invalid-002.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 invalid[1]|
|`upstream/eslint-plugin-n/fixtures/no-sync/invalid-003.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 invalid[2]|
|`upstream/eslint-plugin-n/fixtures/no-sync/invalid-004.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 invalid[3]|
|`upstream/eslint-plugin-n/fixtures/no-sync/invalid-005.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 invalid[4]|
|`upstream/eslint-plugin-n/fixtures/no-sync/invalid-006.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 invalid[5]|
|`upstream/eslint-plugin-n/fixtures/no-sync/invalid-007.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 invalid[6]|
|`upstream/eslint-plugin-n/fixtures/no-sync/invalid-008.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 invalid[7]|
|`upstream/eslint-plugin-n/fixtures/no-sync/invalid-009.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 invalid[8]|
|`upstream/eslint-plugin-n/fixtures/no-sync/invalid-010.js`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 1 invalid[9]|
|`upstream/eslint-plugin-n/fixtures/no-sync/valid-010.ts`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 2 valid[0]|
|`upstream/eslint-plugin-n/fixtures/no-sync/valid-011.ts`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 2 valid[1]|
|`upstream/eslint-plugin-n/fixtures/no-sync/valid-012.ts`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 2 valid[2]|
|`upstream/eslint-plugin-n/fixtures/no-sync/invalid-011.ts`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 2 invalid[0]|
|`upstream/eslint-plugin-n/fixtures/no-sync/invalid-012.ts`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 2 invalid[1]|
|`upstream/eslint-plugin-n/fixtures/no-sync/invalid-013.ts`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 2 invalid[2]|
|`upstream/eslint-plugin-n/fixtures/no-sync/valid-013.ts`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 3 valid[0]|
|`upstream/eslint-plugin-n/fixtures/no-sync/invalid-014.ts`|`tests/lib/rules/no-sync.js`|extracted from tests/lib/rules/no-sync.js run 3 invalid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/buffer/valid-001.js`|`tests/lib/rules/prefer-global/buffer.js`|extracted from tests/lib/rules/prefer-global/buffer.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/buffer/valid-002.js`|`tests/lib/rules/prefer-global/buffer.js`|extracted from tests/lib/rules/prefer-global/buffer.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/buffer/valid-003.js`|`tests/lib/rules/prefer-global/buffer.js`|extracted from tests/lib/rules/prefer-global/buffer.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/buffer/valid-004.js`|`tests/lib/rules/prefer-global/buffer.js`|extracted from tests/lib/rules/prefer-global/buffer.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/buffer/valid-005.js`|`tests/lib/rules/prefer-global/buffer.js`|extracted from tests/lib/rules/prefer-global/buffer.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/buffer/valid-006.js`|`tests/lib/rules/prefer-global/buffer.js`|extracted from tests/lib/rules/prefer-global/buffer.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/buffer/invalid-001.js`|`tests/lib/rules/prefer-global/buffer.js`|extracted from tests/lib/rules/prefer-global/buffer.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/buffer/invalid-002.js`|`tests/lib/rules/prefer-global/buffer.js`|extracted from tests/lib/rules/prefer-global/buffer.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/buffer/invalid-003.js`|`tests/lib/rules/prefer-global/buffer.js`|extracted from tests/lib/rules/prefer-global/buffer.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/buffer/invalid-004.js`|`tests/lib/rules/prefer-global/buffer.js`|extracted from tests/lib/rules/prefer-global/buffer.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/buffer/invalid-005.js`|`tests/lib/rules/prefer-global/buffer.js`|extracted from tests/lib/rules/prefer-global/buffer.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/buffer/invalid-006.js`|`tests/lib/rules/prefer-global/buffer.js`|extracted from tests/lib/rules/prefer-global/buffer.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/buffer/invalid-007.js`|`tests/lib/rules/prefer-global/buffer.js`|extracted from tests/lib/rules/prefer-global/buffer.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/buffer/invalid-008.js`|`tests/lib/rules/prefer-global/buffer.js`|extracted from tests/lib/rules/prefer-global/buffer.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/buffer/invalid-009.js`|`tests/lib/rules/prefer-global/buffer.js`|extracted from tests/lib/rules/prefer-global/buffer.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/console/valid-001.js`|`tests/lib/rules/prefer-global/console.js`|extracted from tests/lib/rules/prefer-global/console.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/console/valid-002.js`|`tests/lib/rules/prefer-global/console.js`|extracted from tests/lib/rules/prefer-global/console.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/console/valid-003.js`|`tests/lib/rules/prefer-global/console.js`|extracted from tests/lib/rules/prefer-global/console.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/console/valid-004.js`|`tests/lib/rules/prefer-global/console.js`|extracted from tests/lib/rules/prefer-global/console.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/console/valid-005.js`|`tests/lib/rules/prefer-global/console.js`|extracted from tests/lib/rules/prefer-global/console.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/console/valid-006.js`|`tests/lib/rules/prefer-global/console.js`|extracted from tests/lib/rules/prefer-global/console.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/console/invalid-001.js`|`tests/lib/rules/prefer-global/console.js`|extracted from tests/lib/rules/prefer-global/console.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/console/invalid-002.js`|`tests/lib/rules/prefer-global/console.js`|extracted from tests/lib/rules/prefer-global/console.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/console/invalid-003.js`|`tests/lib/rules/prefer-global/console.js`|extracted from tests/lib/rules/prefer-global/console.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/console/invalid-004.js`|`tests/lib/rules/prefer-global/console.js`|extracted from tests/lib/rules/prefer-global/console.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/console/invalid-005.js`|`tests/lib/rules/prefer-global/console.js`|extracted from tests/lib/rules/prefer-global/console.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/console/invalid-006.js`|`tests/lib/rules/prefer-global/console.js`|extracted from tests/lib/rules/prefer-global/console.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/console/invalid-007.js`|`tests/lib/rules/prefer-global/console.js`|extracted from tests/lib/rules/prefer-global/console.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/console/invalid-008.js`|`tests/lib/rules/prefer-global/console.js`|extracted from tests/lib/rules/prefer-global/console.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/console/invalid-009.js`|`tests/lib/rules/prefer-global/console.js`|extracted from tests/lib/rules/prefer-global/console.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/process/valid-001.js`|`tests/lib/rules/prefer-global/process.js`|extracted from tests/lib/rules/prefer-global/process.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/process/valid-002.js`|`tests/lib/rules/prefer-global/process.js`|extracted from tests/lib/rules/prefer-global/process.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/process/valid-003.js`|`tests/lib/rules/prefer-global/process.js`|extracted from tests/lib/rules/prefer-global/process.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/process/valid-004.js`|`tests/lib/rules/prefer-global/process.js`|extracted from tests/lib/rules/prefer-global/process.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/process/valid-005.js`|`tests/lib/rules/prefer-global/process.js`|extracted from tests/lib/rules/prefer-global/process.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/process/invalid-001.js`|`tests/lib/rules/prefer-global/process.js`|extracted from tests/lib/rules/prefer-global/process.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/process/invalid-002.js`|`tests/lib/rules/prefer-global/process.js`|extracted from tests/lib/rules/prefer-global/process.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/process/invalid-003.js`|`tests/lib/rules/prefer-global/process.js`|extracted from tests/lib/rules/prefer-global/process.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/process/invalid-004.js`|`tests/lib/rules/prefer-global/process.js`|extracted from tests/lib/rules/prefer-global/process.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/process/invalid-005.js`|`tests/lib/rules/prefer-global/process.js`|extracted from tests/lib/rules/prefer-global/process.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/process/invalid-006.js`|`tests/lib/rules/prefer-global/process.js`|extracted from tests/lib/rules/prefer-global/process.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/process/invalid-007.js`|`tests/lib/rules/prefer-global/process.js`|extracted from tests/lib/rules/prefer-global/process.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/process/invalid-008.js`|`tests/lib/rules/prefer-global/process.js`|extracted from tests/lib/rules/prefer-global/process.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/process/invalid-009.js`|`tests/lib/rules/prefer-global/process.js`|extracted from tests/lib/rules/prefer-global/process.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-decoder/valid-001.js`|`tests/lib/rules/prefer-global/text-decoder.js`|extracted from tests/lib/rules/prefer-global/text-decoder.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-decoder/valid-002.js`|`tests/lib/rules/prefer-global/text-decoder.js`|extracted from tests/lib/rules/prefer-global/text-decoder.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-decoder/valid-003.js`|`tests/lib/rules/prefer-global/text-decoder.js`|extracted from tests/lib/rules/prefer-global/text-decoder.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-decoder/valid-004.js`|`tests/lib/rules/prefer-global/text-decoder.js`|extracted from tests/lib/rules/prefer-global/text-decoder.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-decoder/valid-005.js`|`tests/lib/rules/prefer-global/text-decoder.js`|extracted from tests/lib/rules/prefer-global/text-decoder.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-decoder/valid-006.js`|`tests/lib/rules/prefer-global/text-decoder.js`|extracted from tests/lib/rules/prefer-global/text-decoder.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-decoder/invalid-001.js`|`tests/lib/rules/prefer-global/text-decoder.js`|extracted from tests/lib/rules/prefer-global/text-decoder.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-decoder/invalid-002.js`|`tests/lib/rules/prefer-global/text-decoder.js`|extracted from tests/lib/rules/prefer-global/text-decoder.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-decoder/invalid-003.js`|`tests/lib/rules/prefer-global/text-decoder.js`|extracted from tests/lib/rules/prefer-global/text-decoder.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-decoder/invalid-004.js`|`tests/lib/rules/prefer-global/text-decoder.js`|extracted from tests/lib/rules/prefer-global/text-decoder.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-decoder/invalid-005.js`|`tests/lib/rules/prefer-global/text-decoder.js`|extracted from tests/lib/rules/prefer-global/text-decoder.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-decoder/invalid-006.js`|`tests/lib/rules/prefer-global/text-decoder.js`|extracted from tests/lib/rules/prefer-global/text-decoder.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-decoder/invalid-007.js`|`tests/lib/rules/prefer-global/text-decoder.js`|extracted from tests/lib/rules/prefer-global/text-decoder.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-decoder/invalid-008.js`|`tests/lib/rules/prefer-global/text-decoder.js`|extracted from tests/lib/rules/prefer-global/text-decoder.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-decoder/invalid-009.js`|`tests/lib/rules/prefer-global/text-decoder.js`|extracted from tests/lib/rules/prefer-global/text-decoder.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-encoder/valid-001.js`|`tests/lib/rules/prefer-global/text-encoder.js`|extracted from tests/lib/rules/prefer-global/text-encoder.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-encoder/valid-002.js`|`tests/lib/rules/prefer-global/text-encoder.js`|extracted from tests/lib/rules/prefer-global/text-encoder.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-encoder/valid-003.js`|`tests/lib/rules/prefer-global/text-encoder.js`|extracted from tests/lib/rules/prefer-global/text-encoder.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-encoder/valid-004.js`|`tests/lib/rules/prefer-global/text-encoder.js`|extracted from tests/lib/rules/prefer-global/text-encoder.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-encoder/valid-005.js`|`tests/lib/rules/prefer-global/text-encoder.js`|extracted from tests/lib/rules/prefer-global/text-encoder.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-encoder/valid-006.js`|`tests/lib/rules/prefer-global/text-encoder.js`|extracted from tests/lib/rules/prefer-global/text-encoder.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-encoder/invalid-001.js`|`tests/lib/rules/prefer-global/text-encoder.js`|extracted from tests/lib/rules/prefer-global/text-encoder.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-encoder/invalid-002.js`|`tests/lib/rules/prefer-global/text-encoder.js`|extracted from tests/lib/rules/prefer-global/text-encoder.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-encoder/invalid-003.js`|`tests/lib/rules/prefer-global/text-encoder.js`|extracted from tests/lib/rules/prefer-global/text-encoder.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-encoder/invalid-004.js`|`tests/lib/rules/prefer-global/text-encoder.js`|extracted from tests/lib/rules/prefer-global/text-encoder.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-encoder/invalid-005.js`|`tests/lib/rules/prefer-global/text-encoder.js`|extracted from tests/lib/rules/prefer-global/text-encoder.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-encoder/invalid-006.js`|`tests/lib/rules/prefer-global/text-encoder.js`|extracted from tests/lib/rules/prefer-global/text-encoder.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-encoder/invalid-007.js`|`tests/lib/rules/prefer-global/text-encoder.js`|extracted from tests/lib/rules/prefer-global/text-encoder.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-encoder/invalid-008.js`|`tests/lib/rules/prefer-global/text-encoder.js`|extracted from tests/lib/rules/prefer-global/text-encoder.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/text-encoder/invalid-009.js`|`tests/lib/rules/prefer-global/text-encoder.js`|extracted from tests/lib/rules/prefer-global/text-encoder.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url/valid-001.js`|`tests/lib/rules/prefer-global/url.js`|extracted from tests/lib/rules/prefer-global/url.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url/valid-002.js`|`tests/lib/rules/prefer-global/url.js`|extracted from tests/lib/rules/prefer-global/url.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url/valid-003.js`|`tests/lib/rules/prefer-global/url.js`|extracted from tests/lib/rules/prefer-global/url.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url/valid-004.js`|`tests/lib/rules/prefer-global/url.js`|extracted from tests/lib/rules/prefer-global/url.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url/valid-005.js`|`tests/lib/rules/prefer-global/url.js`|extracted from tests/lib/rules/prefer-global/url.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url/valid-006.js`|`tests/lib/rules/prefer-global/url.js`|extracted from tests/lib/rules/prefer-global/url.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url/invalid-001.js`|`tests/lib/rules/prefer-global/url.js`|extracted from tests/lib/rules/prefer-global/url.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url/invalid-002.js`|`tests/lib/rules/prefer-global/url.js`|extracted from tests/lib/rules/prefer-global/url.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url/invalid-003.js`|`tests/lib/rules/prefer-global/url.js`|extracted from tests/lib/rules/prefer-global/url.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url/invalid-004.js`|`tests/lib/rules/prefer-global/url.js`|extracted from tests/lib/rules/prefer-global/url.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url/invalid-005.js`|`tests/lib/rules/prefer-global/url.js`|extracted from tests/lib/rules/prefer-global/url.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url/invalid-006.js`|`tests/lib/rules/prefer-global/url.js`|extracted from tests/lib/rules/prefer-global/url.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url/invalid-007.js`|`tests/lib/rules/prefer-global/url.js`|extracted from tests/lib/rules/prefer-global/url.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url/invalid-008.js`|`tests/lib/rules/prefer-global/url.js`|extracted from tests/lib/rules/prefer-global/url.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url/invalid-009.js`|`tests/lib/rules/prefer-global/url.js`|extracted from tests/lib/rules/prefer-global/url.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url-search-params/valid-001.js`|`tests/lib/rules/prefer-global/url-search-params.js`|extracted from tests/lib/rules/prefer-global/url-search-params.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url-search-params/valid-002.js`|`tests/lib/rules/prefer-global/url-search-params.js`|extracted from tests/lib/rules/prefer-global/url-search-params.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url-search-params/valid-003.js`|`tests/lib/rules/prefer-global/url-search-params.js`|extracted from tests/lib/rules/prefer-global/url-search-params.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url-search-params/valid-004.js`|`tests/lib/rules/prefer-global/url-search-params.js`|extracted from tests/lib/rules/prefer-global/url-search-params.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url-search-params/valid-005.js`|`tests/lib/rules/prefer-global/url-search-params.js`|extracted from tests/lib/rules/prefer-global/url-search-params.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url-search-params/valid-006.js`|`tests/lib/rules/prefer-global/url-search-params.js`|extracted from tests/lib/rules/prefer-global/url-search-params.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url-search-params/invalid-001.js`|`tests/lib/rules/prefer-global/url-search-params.js`|extracted from tests/lib/rules/prefer-global/url-search-params.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url-search-params/invalid-002.js`|`tests/lib/rules/prefer-global/url-search-params.js`|extracted from tests/lib/rules/prefer-global/url-search-params.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url-search-params/invalid-003.js`|`tests/lib/rules/prefer-global/url-search-params.js`|extracted from tests/lib/rules/prefer-global/url-search-params.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url-search-params/invalid-004.js`|`tests/lib/rules/prefer-global/url-search-params.js`|extracted from tests/lib/rules/prefer-global/url-search-params.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url-search-params/invalid-005.js`|`tests/lib/rules/prefer-global/url-search-params.js`|extracted from tests/lib/rules/prefer-global/url-search-params.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url-search-params/invalid-006.js`|`tests/lib/rules/prefer-global/url-search-params.js`|extracted from tests/lib/rules/prefer-global/url-search-params.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url-search-params/invalid-007.js`|`tests/lib/rules/prefer-global/url-search-params.js`|extracted from tests/lib/rules/prefer-global/url-search-params.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url-search-params/invalid-008.js`|`tests/lib/rules/prefer-global/url-search-params.js`|extracted from tests/lib/rules/prefer-global/url-search-params.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/prefer-global/url-search-params/invalid-009.js`|`tests/lib/rules/prefer-global/url-search-params.js`|extracted from tests/lib/rules/prefer-global/url-search-params.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-001.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-002.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-003.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-004.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-005.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-006.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-007.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[6]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-008.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[7]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-009.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[8]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-010.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[9]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-011.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[10]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-012.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[11]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-013.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[12]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-014.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[13]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-015.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[14]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-016.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[15]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-017.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[16]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-018.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[17]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-019.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[18]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-020.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[19]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-021.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[20]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-022.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[21]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-023.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[22]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-024.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[23]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-025.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[24]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-026.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[25]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-027.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[26]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-028.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[27]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-029.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[28]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-030.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[29]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-031.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[30]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-032.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[31]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-033.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[32]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-034.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[33]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-035.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[34]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-036.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[35]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-037.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[36]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-038.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[37]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-039.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[38]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-040.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[39]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-041.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[40]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-042.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[41]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-043.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[42]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-044.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[43]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/valid-045.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js valid[44]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-001.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-002.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-003.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-004.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-005.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-006.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-007.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-008.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-009.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-010.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[9]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-011.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[10]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-012.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[11]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-013.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[12]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-014.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[13]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-015.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[14]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-016.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[15]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-017.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[16]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-018.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[17]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-019.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[18]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-020.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[19]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-021.js`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[20]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-022.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[21]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-023.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[22]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-024.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[23]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-025.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[24]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-026.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[25]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-027.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[26]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-028.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[27]|
|`upstream/eslint-plugin-n/fixtures/prefer-node-protocol/invalid-029.mjs`|`tests/lib/rules/prefer-node-protocol.js`|extracted from tests/lib/rules/prefer-node-protocol.js invalid[28]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/valid-001.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/valid-002.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/valid-003.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/valid-004.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/valid-005.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/valid-006.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/valid-007.mjs`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js valid[6]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/valid-008.mjs`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js valid[7]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/valid-009.mjs`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js valid[8]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/valid-010.mjs`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js valid[9]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/valid-011.mjs`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js valid[10]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/valid-012.mjs`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js valid[11]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/valid-013.mjs`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js valid[12]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/valid-014.mjs`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js valid[13]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/valid-015.mjs`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js valid[14]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-001.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-002.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-003.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-004.mjs`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-005.mjs`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-006.mjs`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-007.mjs`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-008.mjs`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-009.mjs`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-010.mjs`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[9]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-011.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[10]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-012.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[11]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-013.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[12]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-014.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[13]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-015.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[14]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-016.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[15]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-017.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[16]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-018.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[17]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-019.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[18]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-020.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[19]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-021.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[20]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-022.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[21]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-023.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[22]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-024.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[23]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-025.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[24]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-026.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[25]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/dns/invalid-027.js`|`tests/lib/rules/prefer-promises/dns.js`|extracted from tests/lib/rules/prefer-promises/dns.js invalid[26]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/valid-001.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js valid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/valid-002.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js valid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/valid-003.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js valid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/valid-004.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js valid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/valid-005.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js valid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/valid-006.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js valid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/valid-007.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js valid[6]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/valid-008.mjs`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js valid[7]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/valid-009.mjs`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js valid[8]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/valid-010.mjs`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js valid[9]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/valid-011.mjs`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js valid[10]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/valid-012.mjs`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js valid[11]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/valid-013.mjs`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js valid[12]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/valid-014.mjs`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js valid[13]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/valid-015.mjs`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js valid[14]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/valid-016.mjs`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js valid[15]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-001.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[0]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-002.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[1]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-003.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[2]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-004.mjs`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[3]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-005.mjs`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[4]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-006.mjs`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[5]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-007.mjs`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[6]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-008.mjs`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[7]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-009.mjs`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[8]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-010.mjs`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[9]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-011.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[10]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-012.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[11]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-013.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[12]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-014.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[13]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-015.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[14]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-016.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[15]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-017.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[16]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-018.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[17]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-019.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[18]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-020.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[19]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-021.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[20]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-022.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[21]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-023.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[22]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-024.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[23]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-025.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[24]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-026.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[25]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-027.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[26]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-028.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[27]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-029.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[28]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-030.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[29]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-031.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[30]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-032.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[31]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-033.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[32]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-034.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[33]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-035.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[34]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-036.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[35]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-037.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[36]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-038.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[37]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-039.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[38]|
|`upstream/eslint-plugin-n/fixtures/prefer-promises/fs/invalid-040.js`|`tests/lib/rules/prefer-promises/fs.js`|extracted from tests/lib/rules/prefer-promises/fs.js invalid[39]|
