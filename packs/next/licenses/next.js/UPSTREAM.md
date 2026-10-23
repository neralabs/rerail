# Next.js 16.3.8 (@next/eslint-plugin-next)

Upstream: [vercel/next.js](https://github.com/vercel/next.js/tree/b0fad0d45eb4c4430fda5eeeb442e8a5af08a5f6) at tag `v16.3.8` (commit `b0fad0d45eb4c4430fda5eeeb442e8a5af08a5f6`), the source of `@next/eslint-plugin-next` 16.3.8. License: MIT (Copyright (c) 2025 Vercel, Inc.); [`LICENSE`](LICENSE) is the upstream `license.md`, copied verbatim at that tag.

The files listed below are the plugin's rule tests and the test cases extracted from them, and remain under Next.js's license. Every other file in this pack is Rerail's. The rule documents in `rules/` and `candidates/rules/` are Rerail's re-implementations of @next/eslint-plugin-next's rules, published under the pack's licence expression, `Apache-2.0 AND MIT`, as `REUSE.toml` annotates them. The rest is under the Apache License 2.0 (the pack's `LICENSE`): the recorder `upstream/eslint-plugin-next/generate.mjs` (with its stub `RuleTester` and stub plugin module), `upstream/eslint-plugin-next/fixtures.cases.json`, and the `*.expected.json` outputs recorded by running the pinned plugin.

Paths are relative to the pack directory, `packs/next/`.

| File | Upstream path | Copy |
|---|---|---|
| `upstream/eslint-plugin-next/tests/google-font-display.test.ts` | `test/unit/eslint-plugin-next/google-font-display.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/google-font-preconnect.test.ts` | `test/unit/eslint-plugin-next/google-font-preconnect.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/inline-script-id.test.ts` | `test/unit/eslint-plugin-next/inline-script-id.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/next-script-for-ga.test.ts` | `test/unit/eslint-plugin-next/next-script-for-ga.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-assign-module-variable.test.ts` | `test/unit/eslint-plugin-next/no-assign-module-variable.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-async-client-component.test.ts` | `test/unit/eslint-plugin-next/no-async-client-component.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-before-interactive-script-outside-document.test.ts` | `test/unit/eslint-plugin-next/no-before-interactive-script-outside-document.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-css-tags.test.ts` | `test/unit/eslint-plugin-next/no-css-tags.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-document-import-in-page.test.ts` | `test/unit/eslint-plugin-next/no-document-import-in-page.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-duplicate-head.test.ts` | `test/unit/eslint-plugin-next/no-duplicate-head.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-head-element.test.ts` | `test/unit/eslint-plugin-next/no-head-element.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-head-import-in-document.test.ts` | `test/unit/eslint-plugin-next/no-head-import-in-document.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-img-element.test.ts` | `test/unit/eslint-plugin-next/no-img-element.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-location-assign-relative-destination.test.ts` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-page-custom-font.test.ts` | `test/unit/eslint-plugin-next/no-page-custom-font.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-script-component-in-head.test.ts` | `test/unit/eslint-plugin-next/no-script-component-in-head.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-styled-jsx-in-document.test.ts` | `test/unit/eslint-plugin-next/no-styled-jsx-in-document.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-sync-scripts.test.ts` | `test/unit/eslint-plugin-next/no-sync-scripts.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-title-in-document-head.test.ts` | `test/unit/eslint-plugin-next/no-title-in-document-head.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-typos.test.ts` | `test/unit/eslint-plugin-next/no-typos.test.ts` | verbatim |
| `upstream/eslint-plugin-next/tests/no-unwanted-polyfillio.test.ts` | `test/unit/eslint-plugin-next/no-unwanted-polyfillio.test.ts` | verbatim |
| `upstream/eslint-plugin-next/fixtures/google-font-display/valid-000.js` | `test/unit/eslint-plugin-next/google-font-display.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/google-font-display/valid-001.js` | `test/unit/eslint-plugin-next/google-font-display.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/google-font-display/valid-002.js` | `test/unit/eslint-plugin-next/google-font-display.test.ts` | extracted from valid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/google-font-display/invalid-000.js` | `test/unit/eslint-plugin-next/google-font-display.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/google-font-display/invalid-001.js` | `test/unit/eslint-plugin-next/google-font-display.test.ts` | extracted from invalid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/google-font-display/invalid-002.js` | `test/unit/eslint-plugin-next/google-font-display.test.ts` | extracted from invalid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/google-font-display/invalid-003.js` | `test/unit/eslint-plugin-next/google-font-display.test.ts` | extracted from invalid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/google-font-preconnect/valid-000.js` | `test/unit/eslint-plugin-next/google-font-preconnect.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/google-font-preconnect/invalid-000.js` | `test/unit/eslint-plugin-next/google-font-preconnect.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/google-font-preconnect/invalid-001.js` | `test/unit/eslint-plugin-next/google-font-preconnect.test.ts` | extracted from invalid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/inline-script-id/valid-000.js` | `test/unit/eslint-plugin-next/inline-script-id.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/inline-script-id/valid-001.js` | `test/unit/eslint-plugin-next/inline-script-id.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/inline-script-id/valid-002.js` | `test/unit/eslint-plugin-next/inline-script-id.test.ts` | extracted from valid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/inline-script-id/valid-003.js` | `test/unit/eslint-plugin-next/inline-script-id.test.ts` | extracted from valid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/inline-script-id/valid-004.js` | `test/unit/eslint-plugin-next/inline-script-id.test.ts` | extracted from valid[4] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/inline-script-id/valid-005.js` | `test/unit/eslint-plugin-next/inline-script-id.test.ts` | extracted from valid[5] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/inline-script-id/valid-006.js` | `test/unit/eslint-plugin-next/inline-script-id.test.ts` | extracted from valid[6] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/inline-script-id/valid-007.js` | `test/unit/eslint-plugin-next/inline-script-id.test.ts` | extracted from valid[7] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/inline-script-id/invalid-000.js` | `test/unit/eslint-plugin-next/inline-script-id.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/inline-script-id/invalid-001.js` | `test/unit/eslint-plugin-next/inline-script-id.test.ts` | extracted from invalid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/inline-script-id/invalid-002.js` | `test/unit/eslint-plugin-next/inline-script-id.test.ts` | extracted from invalid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/inline-script-id/invalid-003.js` | `test/unit/eslint-plugin-next/inline-script-id.test.ts` | extracted from invalid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/next-script-for-ga/valid-000.js` | `test/unit/eslint-plugin-next/next-script-for-ga.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/next-script-for-ga/valid-001.js` | `test/unit/eslint-plugin-next/next-script-for-ga.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/next-script-for-ga/valid-002.js` | `test/unit/eslint-plugin-next/next-script-for-ga.test.ts` | extracted from valid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/next-script-for-ga/valid-003.js` | `test/unit/eslint-plugin-next/next-script-for-ga.test.ts` | extracted from valid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/next-script-for-ga/invalid-000.js` | `test/unit/eslint-plugin-next/next-script-for-ga.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/next-script-for-ga/invalid-001.js` | `test/unit/eslint-plugin-next/next-script-for-ga.test.ts` | extracted from invalid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/next-script-for-ga/invalid-002.js` | `test/unit/eslint-plugin-next/next-script-for-ga.test.ts` | extracted from invalid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/next-script-for-ga/invalid-003.js` | `test/unit/eslint-plugin-next/next-script-for-ga.test.ts` | extracted from invalid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-assign-module-variable/valid-000.js` | `test/unit/eslint-plugin-next/no-assign-module-variable.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-assign-module-variable/invalid-000.js` | `test/unit/eslint-plugin-next/no-assign-module-variable.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-async-client-component/valid-000.js` | `test/unit/eslint-plugin-next/no-async-client-component.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-async-client-component/valid-001.js` | `test/unit/eslint-plugin-next/no-async-client-component.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-async-client-component/valid-002.js` | `test/unit/eslint-plugin-next/no-async-client-component.test.ts` | extracted from valid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-async-client-component/valid-003.js` | `test/unit/eslint-plugin-next/no-async-client-component.test.ts` | extracted from valid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-async-client-component/valid-004.js` | `test/unit/eslint-plugin-next/no-async-client-component.test.ts` | extracted from valid[4] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-async-client-component/invalid-000.js` | `test/unit/eslint-plugin-next/no-async-client-component.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-async-client-component/invalid-001.js` | `test/unit/eslint-plugin-next/no-async-client-component.test.ts` | extracted from invalid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-async-client-component/invalid-002.js` | `test/unit/eslint-plugin-next/no-async-client-component.test.ts` | extracted from invalid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-async-client-component/invalid-003.js` | `test/unit/eslint-plugin-next/no-async-client-component.test.ts` | extracted from invalid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-async-client-component/invalid-004.js` | `test/unit/eslint-plugin-next/no-async-client-component.test.ts` | extracted from invalid[4] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-before-interactive-script-outside-document/valid-000/pages/_document.js` | `test/unit/eslint-plugin-next/no-before-interactive-script-outside-document.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-before-interactive-script-outside-document/valid-001/pages/_document.tsx` | `test/unit/eslint-plugin-next/no-before-interactive-script-outside-document.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-before-interactive-script-outside-document/valid-002/pages/_document.tsx` | `test/unit/eslint-plugin-next/no-before-interactive-script-outside-document.test.ts` | extracted from valid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-before-interactive-script-outside-document/valid-003/Users/user_name/projects/project-name/app/layout.tsx` | `test/unit/eslint-plugin-next/no-before-interactive-script-outside-document.test.ts` | extracted from valid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-before-interactive-script-outside-document/valid-004/C/Users/username/projects/project-name/app/layout.tsx` | `test/unit/eslint-plugin-next/no-before-interactive-script-outside-document.test.ts` | extracted from valid[4] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-before-interactive-script-outside-document/valid-005/Users/user_name/projects/project-name/src/app/layout.tsx` | `test/unit/eslint-plugin-next/no-before-interactive-script-outside-document.test.ts` | extracted from valid[5] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-before-interactive-script-outside-document/valid-006/C/Users/username/projects/project-name/src/app/layout.tsx` | `test/unit/eslint-plugin-next/no-before-interactive-script-outside-document.test.ts` | extracted from valid[6] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-before-interactive-script-outside-document/invalid-000/pages/index.js` | `test/unit/eslint-plugin-next/no-before-interactive-script-outside-document.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-before-interactive-script-outside-document/invalid-001/components/outside-known-dirs.js` | `test/unit/eslint-plugin-next/no-before-interactive-script-outside-document.test.ts` | extracted from invalid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-before-interactive-script-outside-document/invalid-002/Users/user_name/projects/project-name/pages/layout.tsx` | `test/unit/eslint-plugin-next/no-before-interactive-script-outside-document.test.ts` | extracted from invalid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-before-interactive-script-outside-document/invalid-003/C/Users/username/projects/project-name/pages/layout.tsx` | `test/unit/eslint-plugin-next/no-before-interactive-script-outside-document.test.ts` | extracted from invalid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-before-interactive-script-outside-document/invalid-004/Users/user_name/projects/project-name/src/pages/layout.tsx` | `test/unit/eslint-plugin-next/no-before-interactive-script-outside-document.test.ts` | extracted from invalid[4] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-before-interactive-script-outside-document/invalid-005/C/Users/username/projects/project-name/src/pages/layout.tsx` | `test/unit/eslint-plugin-next/no-before-interactive-script-outside-document.test.ts` | extracted from invalid[5] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-css-tags/valid-000.js` | `test/unit/eslint-plugin-next/no-css-tags.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-css-tags/valid-001.js` | `test/unit/eslint-plugin-next/no-css-tags.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-css-tags/valid-002.js` | `test/unit/eslint-plugin-next/no-css-tags.test.ts` | extracted from valid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-css-tags/valid-003.js` | `test/unit/eslint-plugin-next/no-css-tags.test.ts` | extracted from valid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-css-tags/invalid-000.js` | `test/unit/eslint-plugin-next/no-css-tags.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-css-tags/invalid-001.js` | `test/unit/eslint-plugin-next/no-css-tags.test.ts` | extracted from invalid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-document-import-in-page/valid-000/pages/_document.js` | `test/unit/eslint-plugin-next/no-document-import-in-page.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-document-import-in-page/valid-001/pages/_document.page.tsx` | `test/unit/eslint-plugin-next/no-document-import-in-page.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-document-import-in-page/valid-002/pages/_document/index.js` | `test/unit/eslint-plugin-next/no-document-import-in-page.test.ts` | extracted from valid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-document-import-in-page/valid-003/pages/_document/index.tsx` | `test/unit/eslint-plugin-next/no-document-import-in-page.test.ts` | extracted from valid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-document-import-in-page/valid-004/pagesapp/src/pages/_document.js` | `test/unit/eslint-plugin-next/no-document-import-in-page.test.ts` | extracted from valid[4] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-document-import-in-page/invalid-000/components/test.js` | `test/unit/eslint-plugin-next/no-document-import-in-page.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-document-import-in-page/invalid-001/pages/test.js` | `test/unit/eslint-plugin-next/no-document-import-in-page.test.ts` | extracted from invalid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-duplicate-head/valid-000/pages/_document.js` | `test/unit/eslint-plugin-next/no-duplicate-head.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-duplicate-head/valid-001/pages/_document.tsx` | `test/unit/eslint-plugin-next/no-duplicate-head.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-duplicate-head/invalid-000/pages/_document.js` | `test/unit/eslint-plugin-next/no-duplicate-head.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-duplicate-head/invalid-001/pages/_document.page.tsx` | `test/unit/eslint-plugin-next/no-duplicate-head.test.ts` | extracted from invalid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-head-element/valid-000/pages/index.js` | `test/unit/eslint-plugin-next/no-head-element.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-head-element/valid-001/pages/index.tsx` | `test/unit/eslint-plugin-next/no-head-element.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-head-element/valid-002/app/layout.js` | `test/unit/eslint-plugin-next/no-head-element.test.ts` | extracted from valid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-head-element/invalid-000/pages/index.js` | `test/unit/eslint-plugin-next/no-head-element.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-head-import-in-document/valid-000/pages/_document.tsx` | `test/unit/eslint-plugin-next/no-head-import-in-document.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-head-import-in-document/valid-001/pages/index.tsx` | `test/unit/eslint-plugin-next/no-head-import-in-document.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-head-import-in-document/invalid-000/pages/_document.js` | `test/unit/eslint-plugin-next/no-head-import-in-document.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-head-import-in-document/invalid-001/pages/_document.page.tsx` | `test/unit/eslint-plugin-next/no-head-import-in-document.test.ts` | extracted from invalid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-head-import-in-document/invalid-002/pages/_document/index.js` | `test/unit/eslint-plugin-next/no-head-import-in-document.test.ts` | extracted from invalid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-head-import-in-document/invalid-003/pages/_document/index.tsx` | `test/unit/eslint-plugin-next/no-head-import-in-document.test.ts` | extracted from invalid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-img-element/valid-000.js` | `test/unit/eslint-plugin-next/no-img-element.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-img-element/valid-001.js` | `test/unit/eslint-plugin-next/no-img-element.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-img-element/valid-002.js` | `test/unit/eslint-plugin-next/no-img-element.test.ts` | extracted from valid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-img-element/valid-003/src/app/icon.js` | `test/unit/eslint-plugin-next/no-img-element.test.ts` | extracted from valid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-img-element/valid-004/app/opengraph-image.tsx` | `test/unit/eslint-plugin-next/no-img-element.test.ts` | extracted from valid[4] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-img-element/invalid-000.js` | `test/unit/eslint-plugin-next/no-img-element.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-img-element/invalid-001.js` | `test/unit/eslint-plugin-next/no-img-element.test.ts` | extracted from invalid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-img-element/invalid-002/some/non-metadata-route-image.tsx` | `test/unit/eslint-plugin-next/no-img-element.test.ts` | extracted from invalid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-000.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-001.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-002.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-003.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-004.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[4] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-005.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[5] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-006.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[6] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-007.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[7] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-008.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[8] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-009.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[9] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-010.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[10] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-011.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[11] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-012.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[12] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-013.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[13] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-014.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[14] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-015.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[15] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-016.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[16] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-017.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[17] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-018.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[18] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-019.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[19] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-020.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[20] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-021.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[21] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-022.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[22] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-023.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[23] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-024.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[24] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-025.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[25] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/valid-026.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from valid[26] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-000.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-001.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-002.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-003.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-004.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[4] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-005.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[5] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-006.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[6] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-007.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[7] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-008.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[8] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-009.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[9] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-010.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[10] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-011.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[11] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-012.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[12] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-013.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[13] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-014.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[14] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-015.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[15] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-016.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[16] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-017.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[17] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-018.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[18] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-019.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[19] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-020.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[20] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-021.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[21] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-022.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[22] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-023.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[23] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-024.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[24] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-025.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[25] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-026.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[26] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-027.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[27] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-location-assign-relative-destination/invalid-028.js` | `test/unit/eslint-plugin-next/no-location-assign-relative-destination.test.ts` | extracted from invalid[28] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-page-custom-font/valid-000/pages/_document.js` | `test/unit/eslint-plugin-next/no-page-custom-font.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-page-custom-font/valid-001/pages/_document.js` | `test/unit/eslint-plugin-next/no-page-custom-font.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-page-custom-font/valid-002/pages/_document.js` | `test/unit/eslint-plugin-next/no-page-custom-font.test.ts` | extracted from valid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-page-custom-font/valid-003/pages/_document.js` | `test/unit/eslint-plugin-next/no-page-custom-font.test.ts` | extracted from valid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-page-custom-font/valid-004/pages/_document.js` | `test/unit/eslint-plugin-next/no-page-custom-font.test.ts` | extracted from valid[4] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-page-custom-font/valid-005/pages/_document.js` | `test/unit/eslint-plugin-next/no-page-custom-font.test.ts` | extracted from valid[5] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-page-custom-font/invalid-000/pages/index.tsx` | `test/unit/eslint-plugin-next/no-page-custom-font.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-page-custom-font/invalid-001/pages/_document.js` | `test/unit/eslint-plugin-next/no-page-custom-font.test.ts` | extracted from invalid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-script-component-in-head/valid-000.js` | `test/unit/eslint-plugin-next/no-script-component-in-head.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-script-component-in-head/invalid-000/pages/index.js` | `test/unit/eslint-plugin-next/no-script-component-in-head.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-styled-jsx-in-document/valid-000/pages/_document.js` | `test/unit/eslint-plugin-next/no-styled-jsx-in-document.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-styled-jsx-in-document/valid-001/pages/_document.js` | `test/unit/eslint-plugin-next/no-styled-jsx-in-document.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-styled-jsx-in-document/valid-002/pages/index.js` | `test/unit/eslint-plugin-next/no-styled-jsx-in-document.test.ts` | extracted from valid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-styled-jsx-in-document/invalid-000/pages/_document.js` | `test/unit/eslint-plugin-next/no-styled-jsx-in-document.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-sync-scripts/valid-000.js` | `test/unit/eslint-plugin-next/no-sync-scripts.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-sync-scripts/valid-001.js` | `test/unit/eslint-plugin-next/no-sync-scripts.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-sync-scripts/invalid-000.js` | `test/unit/eslint-plugin-next/no-sync-scripts.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-sync-scripts/invalid-001.js` | `test/unit/eslint-plugin-next/no-sync-scripts.test.ts` | extracted from invalid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-title-in-document-head/valid-000.js` | `test/unit/eslint-plugin-next/no-title-in-document-head.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-title-in-document-head/valid-001.js` | `test/unit/eslint-plugin-next/no-title-in-document-head.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-title-in-document-head/invalid-000.js` | `test/unit/eslint-plugin-next/no-title-in-document-head.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-typos/valid-000.js` | `test/unit/eslint-plugin-next/no-typos.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-typos/valid-001.js` | `test/unit/eslint-plugin-next/no-typos.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-typos/valid-002.js` | `test/unit/eslint-plugin-next/no-typos.test.ts` | extracted from valid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-typos/valid-003.js` | `test/unit/eslint-plugin-next/no-typos.test.ts` | extracted from valid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-typos/valid-004.js` | `test/unit/eslint-plugin-next/no-typos.test.ts` | extracted from valid[4] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-typos/valid-005.js` | `test/unit/eslint-plugin-next/no-typos.test.ts` | extracted from valid[5] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-typos/invalid-000/pages/index.js` | `test/unit/eslint-plugin-next/no-typos.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-typos/invalid-001/pages/index.js` | `test/unit/eslint-plugin-next/no-typos.test.ts` | extracted from invalid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-typos/invalid-002/pages/index.js` | `test/unit/eslint-plugin-next/no-typos.test.ts` | extracted from invalid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-typos/invalid-003/pages/index.js` | `test/unit/eslint-plugin-next/no-typos.test.ts` | extracted from invalid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-unwanted-polyfillio/valid-000.js` | `test/unit/eslint-plugin-next/no-unwanted-polyfillio.test.ts` | extracted from valid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-unwanted-polyfillio/valid-001.js` | `test/unit/eslint-plugin-next/no-unwanted-polyfillio.test.ts` | extracted from valid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-unwanted-polyfillio/valid-002.js` | `test/unit/eslint-plugin-next/no-unwanted-polyfillio.test.ts` | extracted from valid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-unwanted-polyfillio/valid-003.js` | `test/unit/eslint-plugin-next/no-unwanted-polyfillio.test.ts` | extracted from valid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-unwanted-polyfillio/invalid-000.js` | `test/unit/eslint-plugin-next/no-unwanted-polyfillio.test.ts` | extracted from invalid[0] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-unwanted-polyfillio/invalid-001.js` | `test/unit/eslint-plugin-next/no-unwanted-polyfillio.test.ts` | extracted from invalid[1] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-unwanted-polyfillio/invalid-002.js` | `test/unit/eslint-plugin-next/no-unwanted-polyfillio.test.ts` | extracted from invalid[2] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-unwanted-polyfillio/invalid-003.js` | `test/unit/eslint-plugin-next/no-unwanted-polyfillio.test.ts` | extracted from invalid[3] (its `code`, verbatim) |
| `upstream/eslint-plugin-next/fixtures/no-head-element/2a7f549f6c61.ts` | `test/unit/eslint-plugin-next/no-head-element.test.ts` | derived |

> `recording.json` is a port readiness recording, and no file of this pack, so it is recorded here rather than in the
> table above: the tool-pack's `recording.lock` names it by its sha256, and the bytes are an object in S3, not a file in
> git (docs/port-readiness.md). It carries excerpts of this upstream's tests (`test/unit/eslint-plugin-next/*.test.ts`:
> the `code` of each RuleTester case), redistributed under the licence above, together with Rerail's Apache-2.0;
> `LICENSE` beside this file is this upstream's own, at the pinned commit.

