---
"@neralabs/rerail-pack-node": patch
---

Node.js pack: the rules that read CommonJS's `require`, `module`, `exports`, `__dirname` and `__filename` (`n/exports-style`, `n/no-exports-assign`, `n/no-path-concat`, the `n/prefer-global/*` and `n/prefer-promises/*` rules, and the `n/no-deprecated-api` candidate) no longer treat every file as CommonJS. Each file keeps its own module system (`.cjs`/`.cts` CommonJS; `.mjs`/`.mts`/`.tsx` an ES module; `.js`/`.jsx`/`.ts` by the nearest `package.json`'s `type`, Node's syntax detection where that package.json sets no valid `type`, and an ES module where there is no package.json), as eslint-plugin-n's `flat/mixed-esm-and-cjs` config does, so an ES module is no longer told about `exports`, `require('fs')` or `__dirname`, which Node does not define there. `n/global-require` still reports a nested `require()` in an ES module, as upstream does.
