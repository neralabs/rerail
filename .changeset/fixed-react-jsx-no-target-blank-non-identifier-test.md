---
"@neralabs/rerail-pack-react": patch
---

`react-jsx-no-target-blank` treats `target` and `rel` conditionals whose tests are both something other than a name, such as `url.startsWith("http") ? …`, as one test, as eslint-plugin-react does: only the `rel` branch beside an exact `_blank` must hold `noreferrer`. Such links are no longer reported, and one whose target is `_BLANK` now is.
