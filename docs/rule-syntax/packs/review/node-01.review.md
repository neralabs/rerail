# Review: node-01 (eslint-plugin-n 18.4.1)

Reviewed: `tbelanger/pack-translations-node-01` at 10d74cc3c9: `n-callback-return.rerail`, `n-no-deprecated-api.rerail`
(+ `.facts.rerail`), `n-prefer-promises-fs.rerail`, and the library code they lean on (`lib/tsx/values.rerail`,
`node-trace.rerail`, `node.rerail`, `eslint-a-common.rerail`, `statements.rerail`, `eslint-b-control.rerail`), against
eslint-plugin-n 18.4.1's `lib/rules/callback-return.js`, `no-deprecated-api.js`, `prefer-promises/fs.js`,
`lib/util/iterate-process-get-builtin-module-references.js`, `extend-trackmap-with-node-prefix.js`, and
@eslint-community/eslint-utils 4.10.1's `ReferenceTracker` / `getStaticValue`; upstream docs and tests at f1d47aff.

How the tool was run: on pt-w01, in /tmp, with the node pack generator's pins and configuration
(`packs/node/upstream/eslint-plugin-n/generate.mjs`): eslint 10.11.0, eslint-plugin-n 18.4.1, @typescript-eslint/parser
8.70.1, typescript 5.9.3, globals 17.13.0, Node 24.22.2; source type from the extension, else the nearest package.json's
`type`; `globals.node`, without the CommonJS wrapper names in ES modules; default options; the three rules on. The
translation: `~/pt/bin/rerail-engine run-policies <case> <out> <the three .rerail files>`, from a copy of this commit's
`docs/rule-syntax` (and `native/src/code_rule_dsl/typing/programs`). Each case is a directory with
`package.json: {"name":"<case>","version":"1.0.0"}` (CommonJS) and the files shown. Output lines are `path:line[:col] rule`.

## Summary

| Rule | Verdict |
|---|---|
| n-callback-return | defects D1, D2 (false positives, both in unusual code; no other difference found) |
| n-no-deprecated-api | defects D3, D4, D5, D6, D7, D8; minor D9, D10; note N1 |
| n-prefer-promises-fs | defects D3, D4, D5; minor D10 |

D1 and D2 are in `n-callback-return.rerail`. D3-D7 and D10 are in the value tracing of `lib/tsx/values.rerail`
(ReferenceTracker's counterpart), so they presumably reach every rule built on it (prefer-global/*, prefer-promises/dns,
prefer-node-protocol, no-unsupported-features/node-builtins; not run here). D8 is in names/values (a type-position
`typeof a.b` read as a value), D9 in names.rerail's `import_spec`.

Besides these, the other forms tried agree (cases written for this review, not the plugin's test suite): module loads by
require / `process.getBuiltinModule` / import / `export *` incl. `node:` names; members, computed and template keys,
nested destructuring, defaults in parameters, aliases through `( )`, `,`, `?:`, `||`, `as`, `!`, `?.`;
`globalThis.`/`global.` members; `Intl.v8BreakIterator`, `require.extensions` (and none in an ES module),
`process.env.NODE_REPL_HISTORY_FILE`; `/* global GLOBAL */` and `root:off`; `import type`/`export type`/`import { type x }`;
string-named specifiers; `import x = require()` (neither); `zlib.Deflate()` vs `new zlib.Inflate()`; shadowing and `var`
hoisting; tagged templates; TypeScript with imports in a CommonJS package; and for `n/callback-return`: optional calls,
`x && cb()`, `(cb)()`, `cb(); ;`, `{ cb(); ; return; }`, switch cases, labels, try/catch, member callees, `cb!()`,
`cb<T>()`, `cb() as T`, namespaces, decorators.

## n-callback-return

### D1. A class static block counts as a block (false positives)

ESTree's `StaticBlock` is not a `BlockStatement`, so the plugin's climb (`findClosestParentOfType(node,
["BlockStatement", "ReturnStatement", "ArrowFunctionExpression"])`) passes it. tree-sitter writes the static block's body
as a `statement_block`, so `stop("statement_block")` stops there. Where the class sits in a `return` or an arrow's
expression body, the plugin reaches the `return`/arrow (fine), and the translation the static block (reported).

Case d1, `a.js`:

```
function make(cb) {
  return class { static { cb(); } };
}
const make2 = (next) => class { static { next(); } };
```

- ESLint: no findings.
- Translation: `a.js:2`, `a.js:4` n/callback-return.

Fix: skip a `statement_block` whose parent is `class_static_block` when climbing.

### D2. A computed method key counts as inside the method (false positives)

`inside(c, _, function_node)` holds for a call in a method's computed key, because tree-sitter's `method_definition`
holds the key. In ESTree the key belongs to `MethodDefinition`/`Property`, not to the `FunctionExpression`, so a class or
object literal outside every function has no function above the key, and the plugin reports nothing.

Case d2, `a.js`:

```
const next = () => "key";
class A { [next()]() { return 1; } }
const o = { get [next()]() { return 1; } };
```

- ESLint: no findings.
- Translation: `a.js:2`, `a.js:3` n/callback-return.

(The rule's `[]` option, which turns the rule off in the plugin, cannot be told apart from no option: the engine gives
an empty list no `option_item` rows. Not counted as a defect.)

## Value tracing (n-no-deprecated-api, n-prefer-promises-fs; lib/tsx/values.rerail)

### D3. A default in a destructuring shorthand, `{ x = <traced> }`, is not followed (misses)

ReferenceTracker follows an `AssignmentPattern` whose right side is the traced value into its left binding, wherever the
pattern is: `{ readFile = fs.readFile } = {}` is the usual injectable-fs parameter. values.rerail's assignment rule takes
`assignment_expression` and `assignment_pattern`; tree-sitter writes the shorthand-with-default as
`object_assignment_pattern`, which only the member-key rules know.

Case d3, `a.js`:

```
const fs = require("fs");
function load(file, { readFile = fs.readFile } = {}, done) {
  readFile(file, done);
}
function make({ Alloc = Buffer } = {}) {
  return new Alloc(8);
}
```

- ESLint: `a.js:3:3 n/prefer-promises/fs`, `a.js:6:10 n/no-deprecated-api` ('new Buffer()').
- Translation: no findings.

### D4. Logical assignment (`??=`, `||=`, `&&=`) is not followed (misses)

ESTree's logical assignment is an `AssignmentExpression`, and the tracker follows its right side into the left variable
(and through the expression itself). tree-sitter writes it as `augmented_assignment_expression`; values.rerail's
assignment and `pass_through` rules take only `assignment_expression`.

Case d4, `a.js`:

```
const fs = require("fs");
let stat;
stat ??= fs.stat;
stat(p, done);
let B;
B ||= Buffer;
new B(8);
```

- ESLint: `a.js:4:1 n/prefer-promises/fs`, `a.js:7:1 n/no-deprecated-api` ('new Buffer()').
- Translation: no findings.

### D5. `process.getBuiltinModule` is followed only as the literal member call (misses)

The plugin gets the loader from `tracker.iterateGlobalReferences({ process: { getBuiltinModule: { [CALL]: true } } })`,
so every form the tracker follows loads: a destructured `getBuiltinModule`, an alias, `process["getBuiltinModule"]`.
values.rerail's `loader` asks for a `member_expression` named `getBuiltinModule` on `process`, and nothing makes
`process.getBuiltinModule` a traced value (no `api_known` row), so aliases do not carry it.

Case d5, `a.js`:

```
const { getBuiltinModule } = process;
getBuiltinModule("fs").readFile(p, done);
getBuiltinModule("url").parse(s);
const load = process.getBuiltinModule;
load("fs").mkdir(p, done);
process["getBuiltinModule"]("domain");
```

- ESLint: `a.js:2:1 n/prefer-promises/fs`, `a.js:3:1 n/no-deprecated-api` ('url.parse'), `a.js:5:1 n/prefer-promises/fs`,
  `a.js:6:1 n/no-deprecated-api` ('domain' module).
- Translation: no findings.

### D6. A traced value assigned to a global variable is not followed (misses)

`_iterateLhsReferences` resolves the assigned identifier with `findVariable(globalScope, …)`, which finds the global
`Buffer` that `globals.node` declares, and walks its references. values.rerail carries an assigned value only to a
binding with a declaration site (`ref_site(_, d)`); a global has none. The browserify-era polyfill line is the common
form.

Case d6, `a.js`:

```
Buffer = Buffer || require("buffer").Buffer;
new Buffer(8);
```

- ESLint: `a.js:2:1 n/no-deprecated-api` ('new buffer.Buffer()', through the module; the global itself is not followed,
  being written).
- Translation: no findings.

### D7. A global written by destructuring or `for … of/in` is still followed (false positives)

The tracker skips a global with any write reference (`isModifiedGlobal`). `env_written` sees only `g = …`, `g op= …` and
`g++`, where the left side is an identifier; an object or array pattern, or a `for (g of …)` / `for (g in …)` head, also
writes `g`.

Case d7:

```
a.js:  ({ Buffer } = shim);
       new Buffer(8);
b.js:  for (process of procs) {}
       process.binding("fs");
```

- ESLint: no findings.
- Translation: `a.js:2` ('new Buffer()'), `b.js:2` ('process.binding') n/no-deprecated-api.

### D8. `typeof a.b` in a TypeScript type is read as a member access (false positives)

In typescript-estree, `typeof url.parse` in a type is `TSTypeQuery` over a `TSQualifiedName`, not a `MemberExpression`,
so `_iteratePropertyReferences` stops at `url` and nothing is reported. tree-sitter writes the query's name as a
`member_expression`, and names.rerail resolves its object as a value reference, so `member`/`api_use` report it.
`ReturnType<typeof url.parse>` is ordinary in TypeScript that still uses the legacy API (the call site is reported
anyway; this adds one finding per type).

Case d8, `a.ts`:

```
import * as url from "node:url";
import util from "util";
export let parsed: ReturnType<typeof url.parse>;
export type IsArray = typeof util.isArray;
export type Binding = typeof process.binding;
```

- ESLint: no findings.
- Translation: `a.ts:3` ('url.parse'), `a.ts:4` ('util.isArray'), `a.ts:5` ('process.binding') n/no-deprecated-api.

Fix: no `member` (or no `api_use` read) under a `type_query`.

### D9. A re-export inside `declare module "…" { }` counts (false positive, minor)

`iterateEsmReferences` reads only the program's top-level statements (`programNode.body`). names.rerail's
`import_spec` takes any `import_statement`/`export_statement` with a source, at any depth.

Case d9, `a.d.ts`:

```
declare module "legacy-url" {
  export { parse } from "url";
}
```

- ESLint: no findings.
- Translation: `a.d.ts:2 n/no-deprecated-api` ('url.parse').

### D10. The module name is folded by `getStringIfConstant`, which knows more than `const_str` (misses, minor)

`getStaticValue` (no scope given) also folds logical operators and the safe string methods on constants.
`const_str` folds strings, constant templates, `+` and `true ? a : b` only.

Case d10, `a.js`:

```
require("FS".toLowerCase()).readFile(p, done);
require("fs" || "graceful-fs").stat(p, done);
require("domain" || "x");
```

- ESLint: `a.js:1:1`, `a.js:2:1` n/prefer-promises/fs, `a.js:3:1` n/no-deprecated-api ('domain' module).
- Translation: no findings.

### N1. `node:` names follow the Node that runs ESLint (note, not counted)

`extendTraceMapWithNodePrefix` keeps `node:<m>` where the running Node's `isBuiltin` says so; the translation's
`node_builtin` is Node 26.12's list. On the box's Node 24.22.2, `require("node:_stream_wrap")` is reported
('_stream_wrap' module) and the translation reports nothing; on a Node without `_stream_wrap` both agree.

## n-prefer-promises-fs

Beyond D3, D4, D5, D10: ok. The message drops the function's name, which the plugin gives (`Use
'fs.promises.readFile()' instead.`); messages are free text, so not counted, but the name is cheap to keep
(`module_use`'s `x`).

## The author's result file (node-01.result.json)

`ptcheck --ref tbelanger/pack-translations-node-01 -- --pack node --rules n-callback-return,n-no-deprecated-api,n-prefer-promises-fs`
(pt-w07, ref 10d74cc3c9) reproduces it: n-callback-return same (fixtures 23, rerail-typescript 17), n-no-deprecated-api
same (fixtures 126), n-prefer-promises-fs different (fixtures 0 original-only, 1 translation-only).

- n-prefer-promises-fs, `different`, right `translation`: confirmed. Sample key
  `eslint-plugin-n/prefer-promises/fs/invalid-040.js:1`, `const fs = require('fs'); fs.watch('file', () => {})`: ESLint
  `invalid-040.js:1:27 n/prefer-promises/fs` ('fs.promises.watch()'); translation `invalid-040.js:1`. The reason cites
  eslint-plugin-n 18.6.0, and NOTES-node-01 says the ground truth was eslint 9 with 18.6.0; the pack pins eslint 10.11.0
  and 18.4.1. 18.4.1's `prefer-promises/fs.js` has `watch` too, so the verdict stands; the version in the reason
  should be the pinned one.
- `equivalence.json` is stale for this batch: n-callback-return and n-no-deprecated-api are still listed as `different`
  ("recorded by the suite, not yet explained"), which the suite flags as problems now that both are same, and
  n-prefer-promises-fs's entry still has right `unknown` instead of the result file's reason. (The pack is
  `equivalent: false`, so this does not gate.)
