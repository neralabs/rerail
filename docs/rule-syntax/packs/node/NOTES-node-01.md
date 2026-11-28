# Batch node-01: n-callback-return, n-no-deprecated-api, n-prefer-promises-fs

Ground truth: eslint 9 with eslint-plugin-n 18.6.0 (npx, on a fleet box), `sourceType: "commonjs"`.

## n-callback-return: same

Follows ESLint. A callback call in an optional chain is never "last in its block": ESTree's
`ExpressionStatement.expression` is the `ChainExpression`, not the `CallExpression`, so `isCallbackExpression` is false
and the call is reported. The translation took `callback?.(err)` as the call itself and missed it
(`src/runtime/daemon-control.ts:129, 139` of the rerail-typescript corpus). Now `tail` excludes `optional_link(c)`
(tsx/eslint-b-control). Counterexamples, ESLint's verdicts, all now matched:

```js
function a(callback) { const fail = (e) => { x(); callback?.(e); }; }   // reported
function b(cb) { if (q) { cb?.(1); return; } cb(2); }                  // reported (the call before the return)
function e(cb) { (cb?.(1)); }                                           // reported
function f(cb) { q && cb?.(1); }                                        // reported
function g(cb) { return cb?.(1); }   const h = (cb) => cb?.(1);         // not reported
```

## n-no-deprecated-api: same

Follows ESLint. `GLOBAL; /*globals GLOBAL*/` (fixture invalid-101, engines.node 6.0.0) is reported by ESLint; the
translation missed it because names.rerail's `directive_global` holds for no comment: **engine gap** — the
`eslintDirective` guest tree's nodes have no `value` (probe: `parsed(c, "eslintDirective", r), value(r, v)` and
`value(item, x)` give no rows; `kind`, `text`, `child` do). `directive_named`, `directive_off` and `global_ref` in
tsx/names are empty for the same reason, which affects every rule that trusts `/* global */` comments.

The rule now reads the comment itself (`global_comment`): label from the comment text
(`/\*\s*globals?\s.*`), the name from the item's text up to `:` or space, `off` from the setting's text. Checked
against ESLint on `/*globals GLOBAL*/`, `/* global GLOBAL */`, `/* globals GLOBAL */` (each reported),
`/* global GLOBAL: off, root:readonly */` (only `root`), `// global GLOBAL` and `/* exported GLOBAL */` (neither).

Proposed: the engine exposes the guest nodes' values (`value(r, "global")`, `value(item, "GLOBAL")`,
`value(setting, "off")`), as eslint_directive.rs builds them (`with_value(js_string(..))`). Then `global_comment`
and `item_off` go, and the rule asks `directive_global` again.

## n-prefer-promises-fs: different, translation right

eslint-plugin-n 18.6.0 lists `watch: { [CALL]: true }` in lib/rules/prefer-promises/fs.js and reports
`const fs = require('fs'); fs.watch('file', () => {})` ("Use 'fs.promises.watch()' instead."): upstream's own
invalid-040. The JSON's name regex stops at `statfs`. The translation keeps `watch`. Counts: fixtures 0
original-only, 1 translation-only; rerail-typescript 0/0.

No new library file: the two fixes share no concept.
