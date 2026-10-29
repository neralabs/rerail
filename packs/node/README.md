# Rerail rule pack: Node.js

21 eslint-plugin-n 18.4.1 rules ported to Rerail, with the plugin's default options: `n-no-exports-assign` and
`n-no-process-exit` start on, as the plugin's `flat/recommended` turns them on, and the other 19 start off until turned
on. 3 more ports are candidates ([candidates/README.md](candidates/README.md)). A port starts in
`candidates/rules/` and moves to `rules/` and `pack.json` once it passes the conformance check
([conformance/README.md](../../conformance/README.md#the-gate)). eslint-plugin-n is a gated tool-pack:
`upstream/eslint-plugin-n/gates.yaml` is its upstream manifest (the pins, the file types upstream lints and each rule's
declared upstream facts), so the check judges every port. Each port is scored against the plugin's own findings on its
rule tests and on their rename variants (`holds-under-rename`), and against the labelled lines of real-code samples
([docs/upstream.md](../../docs/upstream.md#samples)), through the engine's production workspace path, with every
disagreement declared in `divergences.json` (one today: `n-prefer-promises-fs` does not report `fs.watch`). The
conformance check (`--check`, run in CI) scores the tests and the samples: it gates the ports in `rules/` and reports
the candidates without gating them.

It follows the [pack structure and validation contract](../../docs/pack-format.md) and is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack node"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): the 21 rules it ships |
| `rules/` | The shipped rules' documents; `node_policies.py` writes every one but the hand-written `n-no-process-exit` |
| `conformance.json` | Each ported rule's upstream rule, how findings compare, and the capabilities it exercises; the pinned plugin |
| `divergences.json` | Declared disagreements with the plugin |
| `upstream/eslint-plugin-n/` | The plugin's upstream manifest (`gates.yaml`), its copied rule tests (`tests/`), one file per test case (`fixtures/`, their rename variants under `fixtures/renamed/`), its expected findings (`fixtures.expected.json`), the generator (`generate.mjs`, sharing `scripts/pack-data/eslint-expected.mjs`), and the lock naming its real-code samples (`samples.lock`, fetched by `npm run fixtures:pull`) with their sources and licences (`SAMPLES.md`) |
| `candidates/` | The 3 candidates (`rules/`: `n-prefer-promises-dns`, listed in `conformance.json`; `n-no-deprecated-api` and `n-hashbang` in `candidates/conformance.json`) and the Node probes (`probes/`), scored, never shipped; `node_policies.py` writes them |
| `candidates/README.md` | What each candidate is waiting on and its rule details; not published |
| `licenses/eslint-plugin-n/` | The plugin's licence and `UPSTREAM.md`, per upstream repository: its source, the patterns of the files taken and how (each file's provenance is `provenance.json` in the tool's test-data archive) |

No rule in `rules/` or `candidates/rules/` pins `languageOptions.sourceType` (the probes do, to state Node facts): each
file keeps its own module system (`.cjs`/`.cts` CommonJS; `.mjs`/`.mts`/`.tsx` an ES module; `.js`/`.jsx`/`.ts` by the
nearest package.json's `type`, Node's syntax detection where that package.json sets no valid `type`, and an ES module
where there is no package.json), as the plugin's `flat/mixed-esm-and-cjs` config picks `flat/recommended-script` or
`flat/recommended-module` per file. Only a CommonJS file has `require`, `module`,
`exports`, `__dirname` and `__filename`; an ES module has none (`flat/recommended-module` turns them off), so the rules
that reach them through scope (exports-style, no-exports-assign, no-path-concat, prefer-global/\*, prefer-promises/\*,
no-deprecated-api) report them in CommonJS files only. Rules upstream reads by name (global-require, no-new-require,
no-mixed-requires, prefer-node-protocol) report in both. The generator lints each fixture the same way:
`globals.node`, without those five names in an ES module. Upstream's RuleTester declares `globals.node` in every case,
so a case it runs as a module that reads one of those names and has no module syntax is written as CommonJS (`.js`
under the fixtures' `"type": "commonjs"` package.json), where Node defines them.

Taking a new snapshot is a maintainer deriving the pack's test data from its pins and publishing it
([docs/upstream.md](../../docs/upstream.md#take-a-new-snapshot)); a pull request carries it only as a publish request
under `upstream/eslint-plugin-n/publish/`. The recipe
(`scripts/pack-data/recipes/node.ts`) runs `upstream/eslint-plugin-n/generate.mjs` against the pinned tools (eslint
10.11.0, eslint-plugin-n 18.4.1, @typescript-eslint/parser 8.70.1, typescript 5.9.3, globals 17.13.0; the same packages
`scripts/pack-data/runtimes/eslint-plugin-n-18.4.1/` pins for the release check and `npm run upstream:bump`).
Regenerating the rule documents runs offline, against the same tools installed under `.rerail/tools/node`:

```sh
npm run fixtures:derive -- --pack node
npm run fixtures:stage -- --pack node    # copy what changed into upstream/eslint-plugin-n/publish/
python3 scripts/generate/packs/node_policies.py
```

Scoring one rule with every false positive and false negative printed, and the candidates and probes:

```sh
node --import tsx conformance/languages/cli.ts --pack node --rule n-no-process-exit --fixtures
node --import tsx conformance/languages/cli.ts --pack-dir packs/node/candidates --fixtures
node --import tsx conformance/languages/cli.ts --pack-dir packs/node/candidates/probes --fixtures
```

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.
Every rule runs with the plugin's default options and reads each file's module system as described above.

### `n-callback-return`

Default callback names: callback, cb, next. A call of one of those is allowed when its nearest block/return/arrow ancestor is a return or an arrow function, when it is the last statement of a function body (or the right side of a binary/logical expression statement there), or when it is the statement just before a block's final return. Calls with a template-string argument are not matched.

### `n-exports-style`

The global `exports` exists only in CommonJS files, so it is reported only there.

### `n-global-require`

Reports a call of an unshadowed `require` with any ancestor outside upstream's ACCEPTABLE_PARENTS (AssignmentExpression, VariableDeclarator, MemberExpression, ExpressionStatement, CallExpression, ConditionalExpression, Program, VariableDeclaration; tree-sitter's arguments and parenthesized_expression have no ESTree node), or inside an optional chain (ESTree ChainExpression). Upstream's isShadowed reads the name only, so a nested `require(…)` in an ES module is reported too.

### `n-handle-callback-err`

Default pattern "err": a function whose first parameter is named `err` and has no reference (read or write). A method is reported at its parameter list, where its ESTree FunctionExpression starts.

### `n-no-callback-literal`

Matches calls of an identifier named `callback` or `cb` (name only, no scope) whose first argument cannot be an error per upstream's couldBeError: a non-null literal, array, object or template literal, or an assignment/sequence/logical/conditional whose possible results all are. The message is softened: callbacks named cb/callback need not be error-first (ws verifyClient, ssh2 authHandler), so it tells the agent to check before changing the call.

### `n-no-exports-assign`

`module.exports = exports = …` and `exports = module.exports = …` are allowed. The global `exports` exists only in CommonJS files, so it is reported only there.

### `n-no-mixed-requires`

Default options, grouping off. A declarator counts as a require when initialised by `require(…)` or a member chain on one (name only, no scope); a chain with `?.` is an ESTree ChainExpression and does not count.

### `n-no-new-require`

Upstream matches by name only (an identifier named `require`), with no scope analysis.

### `n-no-path-concat`

Flags `__dirname`/`__filename` or `import.meta.dirname|filename|url` as the left of `+` or a template substitution when the next part's first character is `/` (path.sep on the generator's POSIX host): a string starting with /, a constant (followed binding), `path.sep` of the path module, or recursively the left of a binary, either side of ||/&&/??, either branch of ?:, an assignment's right side, or a template's first part. `__dirname`/`__filename` concatenation is caught only in CommonJS files (the `-commonjs` clauses); ES modules are checked for `import.meta` paths.

### `n-no-process-env`

Upstream matches by name only (an identifier named `process`), with no scope analysis. The default options allow no variable.

### `n-no-process-exit`

Upstream matches by name only (an identifier named `process`), with no scope analysis. Written by hand, not generated by `node_policies.py`.

### `n-no-sync`

Upstream selectors `CallExpression > MemberExpression.callee Identifier[name=/Sync$/]` and `CallExpression > Identifier[name=/Sync$/]`, reported at the identifier's ESTree parent: a call with a /Sync$/ identifier as callee or argument, or a node inside (or being) a call's member callee with a /Sync$/ identifier child. So a `…Sync` identifier passed as an argument is also reported.

### `n-prefer-global-buffer`

Default "always". References to `Buffer` through the `buffer` module are found as ReferenceTracker (legacy mode) does: `require('buffer')`/`require('node:buffer')` with the global require, `process.getBuiltinModule(…)` with the global process, through constant aliases, member reads, object-pattern properties, import/export specifiers and `export *`. `require(…)` is reached only in CommonJS files; in ES modules, only imports and `process.getBuiltinModule(…)` are.

### `n-prefer-global-console`

Default "always". References to `console` through the `console` module are found as ReferenceTracker (legacy mode) does: `require('console')`/`require('node:console')` with the global require, `process.getBuiltinModule(…)` with the global process, through constant aliases, member reads, object-pattern properties, import/export specifiers and `export *`. The module itself and every ESM import/export declaration from it are reported. `require(…)` is reached only in CommonJS files; in ES modules, only imports and `process.getBuiltinModule(…)` are.

### `n-prefer-global-process`

Default "always". References to `process` through the `process` module are found as ReferenceTracker (legacy mode) does: `require('process')`/`require('node:process')` with the global require, `process.getBuiltinModule(…)` with the global process, through constant aliases, member reads, object-pattern properties, import/export specifiers and `export *`. The module itself and every ESM import/export declaration from it are reported. `require(…)` is reached only in CommonJS files; in ES modules, only imports and `process.getBuiltinModule(…)` are.

### `n-prefer-global-text-decoder`

Default "always". References to `TextDecoder` through the `util` module are found as ReferenceTracker (legacy mode) does: `require('util')`/`require('node:util')` with the global require, `process.getBuiltinModule(…)` with the global process, through constant aliases, member reads, object-pattern properties, import/export specifiers and `export *`. `require(…)` is reached only in CommonJS files; in ES modules, only imports and `process.getBuiltinModule(…)` are.

### `n-prefer-global-text-encoder`

Default "always". References to `TextEncoder` through the `util` module are found as ReferenceTracker (legacy mode) does: `require('util')`/`require('node:util')` with the global require, `process.getBuiltinModule(…)` with the global process, through constant aliases, member reads, object-pattern properties, import/export specifiers and `export *`. `require(…)` is reached only in CommonJS files; in ES modules, only imports and `process.getBuiltinModule(…)` are.

### `n-prefer-global-url`

Default "always". References to `URL` through the `url` module are found as ReferenceTracker (legacy mode) does: `require('url')`/`require('node:url')` with the global require, `process.getBuiltinModule(…)` with the global process, through constant aliases, member reads, object-pattern properties, import/export specifiers and `export *`. `require(…)` is reached only in CommonJS files; in ES modules, only imports and `process.getBuiltinModule(…)` are.

### `n-prefer-global-url-search-params`

Default "always". References to `URLSearchParams` through the `url` module are found as ReferenceTracker (legacy mode) does: `require('url')`/`require('node:url')` with the global require, `process.getBuiltinModule(…)` with the global process, through constant aliases, member reads, object-pattern properties, import/export specifiers and `export *`. `require(…)` is reached only in CommonJS files; in ES modules, only imports and `process.getBuiltinModule(…)` are.

### `n-prefer-node-protocol`

Flags a string naming a module in the plugin's NodeBuiltinModules table (lib/unsupported-features/node-builtins.js) that also has a `node:<name>` entry, used as the only argument of a non-optional `require(…)`, the first argument of `process.getBuiltinModule(…)`/`globalThis.process.getBuiltinModule(…)`, an import/export source, or an `import(…)` argument. The file's Node range (nearest package.json `engines.node`, else its `devEngines.runtime` entry named node, else >=16.0.0) must be a subset of ^12.20.0 || >=14.13.1 for imports and ^14.18.0 || >=16.0.0 for require; process.getBuiltinModule is checked regardless. Not ported: the `version` option and the `n`/`node` settings that override the range.

### `n-prefer-promises-fs`

Callback API covered: access, copyFile, open, rename, truncate, rmdir, mkdir, readdir, readlink, symlink, lstat, stat, link, unlink, chmod, lchmod, lchown, chown, utimes, realpath, mkdtemp, writeFile, appendFile, readFile, cp, glob, lutimes, opendir, rm, statfs. Found as ReferenceTracker (legacy mode) does, via require, process.getBuiltinModule, ESM default/namespace imports, constant aliases, destructuring and import specifiers. Unlike ReferenceTracker it does not follow assignments to imported or const bindings (those throw at runtime). `require(…)` is reached only in CommonJS files; in ES modules, only imports and `process.getBuiltinModule(…)` are. `watch` is intentionally not reported (fs.promises.watch is an async iterator, not a drop-in FSWatcher replacement).
