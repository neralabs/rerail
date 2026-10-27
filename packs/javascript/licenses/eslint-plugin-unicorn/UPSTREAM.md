# eslint-plugin-unicorn 76.0.0

Upstream: [eslint-plugin-unicorn 76.0.0](https://github.com/sindresorhus/eslint-plugin-unicorn/tree/v76.0.0) (commit `f70155ebe24dbb35b9814c3dbb3ab3620e09c3e1`). License: MIT (Copyright (c) Sindre Sorhus <sindresorhus@gmail.com> (https://sindresorhus.com)); [`LICENSE`](LICENSE) is the upstream license file (`license` in the repository), copied verbatim at that tag.

The files listed below contain material copied from eslint-plugin-unicorn and remain under its license. The rule documents in
`candidates/rules/unicorn-*.json` are Rerail's re-implementations of four of the plugin's rules, under the pack's
licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The recorder `upstream/eslint-plugin-unicorn/generate.mjs` and
the `fixtures.expected.json` recording it writes by running the plugin are Rerail's, under the Apache License 2.0 (the pack's `LICENSE`).

Each copied file is the `code` of one of the rule's own `valid` or `invalid` test cases, verbatim, written by the
recorder: cases for another parser or file type are left out, and identical cases are written once. The port readiness gates'
recorder (`npm run port-readiness:record`) reuses those files and writes every other case the gates read, the same verbatim
`code`, as `upstream/eslint-plugin-unicorn/fixtures/<rule>/<sha256 prefix>.<ext>`. Its recording (`recording.tar.zst`,
which `upstream/eslint-plugin-unicorn/recording.lock` names by its sha256; it is an object in S3, not committed) also holds the `code` of every
test case of the plugin's rule tests, verbatim, as the words `not-overfit` reads.

Paths are relative to the pack directory, `packs/javascript/`.

| File | Upstream path | Copy |
|---|---|---|
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/001/src/foo/bar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/002/src/foo/fooBar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/003/src/foo/bar.test.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/004/src/foo/fooBar.test.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/005/src/foo/fooBar.test-utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/006/src/foo/fooBar.test_utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/007/src/foo/.test_utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/008/src/foo/innerHTML.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/009/src/foo/getDOMRangeRect.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/010/src/foo/apiURL.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/011/src/foo/getHTML5Parser.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/012/src/foo/domSelection.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/013/src/getDOMRangeRect/file.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/014/src/foo/foo.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/015/src/foo/foo_bar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/016/src/foo/foo.test.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/017/src/foo/foo_bar.test.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/018/src/foo/foo_bar.test_utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/019/src/foo/foo_bar.test-utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/020/src/foo/.test-utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/021/src/foo/foo-bar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/022/src/foo/foo-bar.test.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/023/src/foo/foo-bar.test-utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/024/src/foo/foo-bar.test_utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/025/Src/Foo/Foo.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/026/Src/Foo/FooBar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/027/Src/Foo/FAQPage.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/028/Src/Foo/DIYWidget.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/029/Src/Foo/URL2Path.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/030/Src/Foo/FAQI18n.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/031/Src/Foo/URL2I18n.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/032/Src/FAQPage/Foo.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/033/Src/URL2Path/Foo.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/034/Src/URL2I18n/Foo.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/035/Src/Foo/Foo.test.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/036/Src/Foo/FooBar.test.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/037/Src/Foo/FooBar.test-utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/038/Src/Foo/FooBar.test_utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/039/Src/Foo/.test_utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/040/spec/iss47Spec.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/041/spec/iss47Spec100.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/042/spec/i18n.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/043/spec/iss47-spec.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/044/spec/iss-47-spec.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/045/spec/iss47-100spec.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/046/spec/iss47_spec.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/047/spec/iss_47_spec.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/048/spec/iss47_100spec.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/049/Spec/Iss47Spec.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/050/Spec/Iss47.100spec.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/051/Spec/I18n.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/052.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/053/src/foo/_fooBar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/054/src/foo/___fooBar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/055/src/foo/_foo_bar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/056/src/foo/___foo_bar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/057/src/foo/_foo-bar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/058/src/foo/___foo-bar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/059/Src/Foo/_FooBar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/060/Src/Foo/___FooBar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/061/src/foo/$foo.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/062/src/foo/$userId.tsx` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/063/src/foo/$foo_bar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/064/src/foo/$fooBar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/065/src/foo/$idCertidao.tsx` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/066/src/foo/[fooBar].js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/067/src/foo/{foo_bar}.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/068/src/foo/index.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/069/src/foo/FOOBAR.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/070/src/foo/BARBAZ.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/071/src/foo/[FOOBAR].js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/072/src/foo/{FOOBAR}.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/073/src/foo/FOO_bar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/074/src/foo/FooBar.web.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/075/src/foo/FooBar.android.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/076/src/foo/FooBar.ios.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/077/src/foo/FooBar.something.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/078/src/foo/FooBar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/079/src/foo/BaRbAz.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/080/index.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/081/index.mjs` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/082/index.cjs` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/083/index.ts` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/084/index.tsx` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/085/Src/Index/index.tsx` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/086/src/foo/fooBar.testUtils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/087/src/foo/$userId.test.tsx` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/088/Src/Foo/Foo.Test.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/089/Src/Foo/FooBar.Test.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/090/Src/Foo/FooBar.TestUtils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/091/Spec/Iss47.100Spec.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/092/src/foo/Ab.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/093/src/foo/aBc.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/094/src/foo/1Ab.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/095/src/foo/foo_bar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/096/src/foo/fooBar.Test.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/097/test/foo/fooBar.testUtils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/098/test/foo/.testUtils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/099/test/foo/foo_bar.Test.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/100/test/foo/foo_bar.Test_Utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/101/test/foo/.Test_Utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/102/test/foo/foo-bar.Test.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/103/test/foo/foo-bar.Test-Utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/104/test/foo/.Test-Utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/105/Test/Foo/FooBar.Test.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/106/Test/Foo/FooBar.TestUtils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/107/Test/Foo/.TestUtils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/108/src/foo-bar/file.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/109/src/$userId/page.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/110/src/FooBar/file.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/111/src/meta/BadName.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/112/work/Src/fooBar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/113/src/foo/foo_bar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/114/test/foo/foo_bar.test_utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/115/test/foo/fooBar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/116/test/foo/fooBar.test.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/117/src/foo/Article.ts` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/118/Test/Foo/fooBar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/119/Test/Foo/foo_bar.test.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/120/Test/Foo/foo-bar.test-utils.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/121/Src/Foo/PageFAQ.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/122/Src/Foo/FAQ-Page.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/123/Src/Foo/FAQpage.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/124/Src/Foo/FAQPageFOO.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/125/Src/FAQpage/Foo.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/126/Src/Foo/URL2path.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/127/Src/Foo/UIPath.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/128/Src/Foo/UI2Path.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/129/Src/Foo/FOO2.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/130/src/foo/FAQPage.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/131/src/foo/HTMLParser.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/132/src/foo/XMLHttpRequest.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/133/src/foo/_FOO-BAR.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/134/src/foo/___FOO-BAR.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/135/Src/Foo/_FOO-BAR.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/136/Src/Foo/___FOO-BAR.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/137/src/foo/[foo_bar].{baz_qux}.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/138/src/_FOO-BAR/file.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/139/src/$UserId/fooBar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/140/src/FooBar/foo_bar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/141/src/foo-bar/foo_bar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/142/src/FooBar/index.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/143/work/project/src/FooBar/file.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/144/src/foo/[foo_bar].js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/145/src/foo/foo$Bar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/146/src/foo/barBaz.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/147/src/qux/FooBar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/148/src/foo/1_.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/149/src/foo/a_b.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/150/src/foo/f_a_q_page.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/151/src/foo/a_b_c.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/152/src/foo/1_a_b.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/153.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/154/src/foo-js/bar.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/155/src/foo-js/bar.spec.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/156/src/foo-js/.spec.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/157/foo.SPEC.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/filename-case/158/.SPEC.js` | `test/filename-case.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/001.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/002.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/003.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/004.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/005.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/006.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/007.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/008.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/009.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/010.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/011.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/012.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/013.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/014.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/015.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/016.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/017.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/018.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/019.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/020.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/021.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/022.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/023.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/024.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/025.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/026.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/027.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/028.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/029.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/030.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/031.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/032.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/033.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/034.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/035.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/036.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/037.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/038.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/039.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/040.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/041.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/042.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/043.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/044.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/045.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/046.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/047.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/048.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/049.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/050.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/051.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/052.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/053.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/054.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/055.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/056.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/057.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/no-null/058.js` | `test/no-null.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/001.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/002.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/003.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/004.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/005.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/006.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/007.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/008.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/009.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/010.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/011.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/012.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/013.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/014.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/015.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/016.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/017.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/018.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/019.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/020.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/021.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/022.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/023.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/024.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/025.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/026.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/027.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/028.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/029.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/030.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/031.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/032.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/033.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/034.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/035.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/036.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/037.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/038.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/039.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/040.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/041.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/042.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/043.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/044.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/045.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/046.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/047.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/048.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/049.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/050.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/051.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/052.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/053.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/054.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/055.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/056.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/057.js` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/058.ts` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/059.ts` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/060.ts` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/061.ts` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/062.ts` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/063.ts` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/064.ts` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/prefer-node-protocol/065.ts` | `test/prefer-node-protocol.js` | derived |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/filename-case/095/src/foo/foo_bar.js` | `test/filename-case.js` | derived (`fixtures/filename-case/095/src/foo/foo_bar.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/filename-case/113/src/foo/foo_bar.js` | `test/filename-case.js` | derived (`fixtures/filename-case/113/src/foo/foo_bar.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/filename-case/153.js` | `test/filename-case.js` | derived (`fixtures/filename-case/153.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/filename-case/154/src/foo-js/bar.js` | `test/filename-case.js` | derived (`fixtures/filename-case/154/src/foo-js/bar.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/filename-case/155/src/foo-js/bar.spec.js` | `test/filename-case.js` | derived (`fixtures/filename-case/155/src/foo-js/bar.spec.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/filename-case/156/src/foo-js/.spec.js` | `test/filename-case.js` | derived (`fixtures/filename-case/156/src/foo-js/.spec.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/filename-case/157/foo.SPEC.js` | `test/filename-case.js` | derived (`fixtures/filename-case/157/foo.SPEC.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/filename-case/158/.SPEC.js` | `test/filename-case.js` | derived (`fixtures/filename-case/158/.SPEC.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/001.js` | `test/no-null.js` | derived (`fixtures/no-null/001.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/003.js` | `test/no-null.js` | derived (`fixtures/no-null/003.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/004.js` | `test/no-null.js` | derived (`fixtures/no-null/004.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/005.js` | `test/no-null.js` | derived (`fixtures/no-null/005.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/006.js` | `test/no-null.js` | derived (`fixtures/no-null/006.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/008.js` | `test/no-null.js` | derived (`fixtures/no-null/008.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/012.js` | `test/no-null.js` | derived (`fixtures/no-null/012.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/013.js` | `test/no-null.js` | derived (`fixtures/no-null/013.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/014.js` | `test/no-null.js` | derived (`fixtures/no-null/014.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/015.js` | `test/no-null.js` | derived (`fixtures/no-null/015.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/016.js` | `test/no-null.js` | derived (`fixtures/no-null/016.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/017.js` | `test/no-null.js` | derived (`fixtures/no-null/017.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/018.js` | `test/no-null.js` | derived (`fixtures/no-null/018.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/019.js` | `test/no-null.js` | derived (`fixtures/no-null/019.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/020.js` | `test/no-null.js` | derived (`fixtures/no-null/020.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/021.js` | `test/no-null.js` | derived (`fixtures/no-null/021.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/022.js` | `test/no-null.js` | derived (`fixtures/no-null/022.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/023.js` | `test/no-null.js` | derived (`fixtures/no-null/023.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/024.js` | `test/no-null.js` | derived (`fixtures/no-null/024.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/025.js` | `test/no-null.js` | derived (`fixtures/no-null/025.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/026.js` | `test/no-null.js` | derived (`fixtures/no-null/026.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/027.js` | `test/no-null.js` | derived (`fixtures/no-null/027.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/028.js` | `test/no-null.js` | derived (`fixtures/no-null/028.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/029.js` | `test/no-null.js` | derived (`fixtures/no-null/029.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/030.js` | `test/no-null.js` | derived (`fixtures/no-null/030.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/031.js` | `test/no-null.js` | derived (`fixtures/no-null/031.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/032.js` | `test/no-null.js` | derived (`fixtures/no-null/032.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/033.js` | `test/no-null.js` | derived (`fixtures/no-null/033.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/034.js` | `test/no-null.js` | derived (`fixtures/no-null/034.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/035.js` | `test/no-null.js` | derived (`fixtures/no-null/035.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/036.js` | `test/no-null.js` | derived (`fixtures/no-null/036.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/037.js` | `test/no-null.js` | derived (`fixtures/no-null/037.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/038.js` | `test/no-null.js` | derived (`fixtures/no-null/038.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/040.js` | `test/no-null.js` | derived (`fixtures/no-null/040.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/042.js` | `test/no-null.js` | derived (`fixtures/no-null/042.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/044.js` | `test/no-null.js` | derived (`fixtures/no-null/044.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/046.js` | `test/no-null.js` | derived (`fixtures/no-null/046.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/048.js` | `test/no-null.js` | derived (`fixtures/no-null/048.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/049.js` | `test/no-null.js` | derived (`fixtures/no-null/049.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/050.js` | `test/no-null.js` | derived (`fixtures/no-null/050.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/051.js` | `test/no-null.js` | derived (`fixtures/no-null/051.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/053.js` | `test/no-null.js` | derived (`fixtures/no-null/053.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/054.js` | `test/no-null.js` | derived (`fixtures/no-null/054.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/055.js` | `test/no-null.js` | derived (`fixtures/no-null/055.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/056.js` | `test/no-null.js` | derived (`fixtures/no-null/056.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/057.js` | `test/no-null.js` | derived (`fixtures/no-null/057.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/no-null/058.js` | `test/no-null.js` | derived (`fixtures/no-null/058.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/001.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/001.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/002.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/002.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/003.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/003.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/004.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/004.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/005.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/005.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/006.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/006.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/007.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/007.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/017.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/017.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/018.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/018.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/020.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/020.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/021.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/021.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/022.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/022.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/024.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/024.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/025.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/025.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/026.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/026.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/027.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/027.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/028.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/028.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/029.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/029.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/033.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/033.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/034.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/034.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/035.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/035.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/036.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/036.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/037.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/037.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/038.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/038.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/039.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/039.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/040.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/040.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/041.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/041.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/042.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/042.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/043.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/043.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/044.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/044.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/045.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/045.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/046.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/046.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/047.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/047.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/048.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/048.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/049.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/049.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/050.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/050.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/051.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/051.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/052.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/052.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/053.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/053.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/054.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/054.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/055.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/055.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/056.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/056.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/057.js` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/057.js` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/058.ts` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/058.ts` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/059.ts` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/059.ts` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/060.ts` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/060.ts` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/061.ts` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/061.ts` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/062.ts` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/062.ts` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/063.ts` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/063.ts` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/064.ts` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/064.ts` with its names and literals renamed) |
| `upstream/eslint-plugin-unicorn/fixtures/renamed/prefer-node-protocol/065.ts` | `test/prefer-node-protocol.js` | derived (`fixtures/prefer-node-protocol/065.ts` with its names and literals renamed) |

> `recording.tar.zst` is a port readiness recording, and no file of this pack, so it is recorded here rather than in the
> table above: the tool-pack's `recording.lock` names it by its sha256, and the bytes are an object in S3, not a file in
> git (docs/port-readiness.md). It carries excerpts of this upstream's tests (`test/*.js`: the `code` of each RuleTester
> case), redistributed under the licence above, together with Rerail's Apache-2.0; `LICENSE` beside this file is this
> upstream's own, at the pinned commit.

