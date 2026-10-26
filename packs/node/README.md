# Rerail rule pack: Node.js

24 ports of eslint-plugin-n 18.4.1 rules with its default options. A port starts in `candidates/rules/` and moves to
`rules/` and `pack.json` once it passes the port readiness gates ([docs/port-readiness.md](../../docs/port-readiness.md)); eslint-plugin-n
has no port readiness recording yet, so every port is a candidate today, `pack.json` lists no rules, and the pack ships
none and is never recommended. Each port is scored against the plugin's own findings on its rule tests, and against the
labelled lines of real-code samples ([docs/port-readiness.md](../../docs/port-readiness.md#samples)), through the
engine's production workspace path, with every disagreement declared in `divergences.json` (one today:
`n-prefer-promises-fs` does not report `fs.watch`). The conformance check (`--check`, run in CI) scores the tests and
the samples: it gates the ports in `rules/` and reports the candidates without gating them.

It follows the [pack structure and validation contract](../../docs/pack-format.md) and is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack node"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`); it lists no rules while every port is a candidate |
| `conformance.json` | Each ported rule's upstream rule, how findings compare, and the capabilities it exercises; the pinned plugin |
| `divergences.json` | Declared disagreements with the plugin |
| `upstream/eslint-plugin-n/` | The plugin's copied rule tests (`tests/`), one file per test case (`fixtures/`), its recording (`fixtures.expected.json`), the recorder (`generate.mjs`, sharing `conformance/recorders/eslint-record.mjs`), and the lock naming its real-code samples (`samples.lock`, fetched by `npm run fixtures:pull`) with their sources and licences (`SAMPLES.md`) |
| `candidates/` | Every port (`rules/`, listed in `conformance.json`; `n-no-deprecated-api` and `n-hashbang` in `candidates/conformance.json`) and the Node probes (`probes/`), scored, never shipped; `node_policies.py` writes every document but the hand-written `n-no-process-exit` |
| `licenses/eslint-plugin-n/` | The plugin's licence and `UPSTREAM.md`, listing every copied file |

No rule in `rules/` or `candidates/rules/` pins `languageOptions.sourceType` (the probes do, to state Node facts): each
file keeps its own module system (`.cjs`/`.cts` CommonJS; `.mjs`/`.mts`/`.tsx` an ES module; `.js`/`.jsx`/`.ts` by the
nearest package.json's `type`, Node's syntax detection where that package.json sets no valid `type`, and an ES module
where there is no package.json), as the plugin's `flat/mixed-esm-and-cjs` config picks `flat/recommended-script` or
`flat/recommended-module` per file. Only a CommonJS file has `require`, `module`,
`exports`, `__dirname` and `__filename`; an ES module has none (`flat/recommended-module` turns them off), so the rules
that reach them through scope (exports-style, no-exports-assign, no-path-concat, prefer-global/\*, prefer-promises/\*,
no-deprecated-api) report them in CommonJS files only. Rules upstream reads by name (global-require, no-new-require,
no-mixed-requires, prefer-node-protocol) report in both. The recorder lints each fixture the same way:
`globals.node`, without those five names in an ES module. Upstream's RuleTester declares `globals.node` in every case,
so a case it runs as a module that reads one of those names and has no module syntax is written as CommonJS (`.js`
under the fixtures' `"type": "commonjs"` package.json), where Node defines them.

Re-recording is a maintainer deriving the pack's test data from its pins and archiving it
([CONTRIBUTING.md](../../CONTRIBUTING.md#pack-test-data)); it is never committed. The recipe
(`scripts/pack-data/recipes/node.ts`) runs `upstream/eslint-plugin-n/generate.mjs` against the pinned tools (eslint
10.11.0, eslint-plugin-n 18.4.1, @typescript-eslint/parser 8.70.1, typescript 5.9.3, globals 17.13.0).
Regenerating the rule documents runs offline, against the same tools installed under `.rerail/tools/node`:

```sh
npm run fixtures:derive -- --pack node
npm run fixtures:push -- --pack node     # on your own branch: archive it and rewrite its test-data.lock
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

### Module system per file (shared by n-exports-style, n-global-require, n-no-exports-assign, n-no-path-concat, n-prefer-global-*, n-prefer-promises-*)

All ports target eslint-plugin-n 18.4.1 with default options. No sourceType is pinned, so each file keeps its own module system: .cjs/.cts are CommonJS; .mjs/.mts/.tsx are ES modules; .js/.jsx/.ts follow the nearest package.json's `type`, fall back to Node's syntax detection when that package.json sets no valid type, and are ES modules when there is no package.json. Like the plugin's flat/mixed-esm-and-cjs config, a CommonJS file has the globals require, module, exports, __dirname and __filename (flat/recommended-script) and an ES module has none (flat/recommended-module). Consequences: the global `exports` exists only in CommonJS files; `__dirname`/`__filename` path concatenation is caught only in CommonJS (the `-commonjs` clauses), while ES modules are checked for `import.meta` paths; for prefer-global and prefer-promises rules, `require(…)` is reached only in CommonJS files, and in ES modules only imports and `process.getBuiltinModule(…)` are.

### `n-callback-return`

Default callback names: callback, cb, next. A call of one of those is allowed when its nearest block/return/arrow ancestor is a return or an arrow function, when it is the last statement of a function body (or the right side of a binary/logical expression statement there), or when it is the statement just before a block's final return. Calls with a template-string argument are not matched.

### `n-global-require`

Reports a call of an unshadowed `require` with any ancestor outside upstream's ACCEPTABLE_PARENTS (AssignmentExpression, VariableDeclarator, MemberExpression, ExpressionStatement, CallExpression, ConditionalExpression, Program, VariableDeclaration; tree-sitter's arguments and parenthesized_expression have no ESTree node), or inside an optional chain (ESTree ChainExpression). Upstream's isShadowed reads the name only, so a nested `require(…)` in an ES module is reported too.

### `n-handle-callback-err`

Default pattern "err": a function whose first parameter is named `err` and has no reference (read or write). A method is reported at its parameter list, where its ESTree FunctionExpression starts.

### `n-no-callback-literal`

Matches calls of an identifier named `callback` or `cb` (name only, no scope) whose first argument cannot be an error per upstream's couldBeError: a non-null literal, array, object or template literal, or an assignment/sequence/logical/conditional whose possible results all are. The message is softened: callbacks named cb/callback need not be error-first (ws verifyClient, ssh2 authHandler), so it tells the agent to check before changing the call.

### `n-no-exports-assign`

`module.exports = exports = …` and `exports = module.exports = …` are allowed.

### `n-no-mixed-requires`

Default options, grouping off. A declarator counts as a require when initialised by `require(…)` or a member chain on one (name only, no scope); a chain with `?.` is an ESTree ChainExpression and does not count.

### `n-no-path-concat`

Flags `__dirname`/`__filename` or `import.meta.dirname|filename|url` as the left of `+` or a template substitution when the next part's first character is `/` (path.sep on the recorder's POSIX host): a string starting with /, a constant (followed binding), `path.sep` of the path module, or recursively the left of a binary, either side of ||/&&/??, either branch of ?:, an assignment's right side, or a template's first part.

### `n-no-process-env` / `n-no-process-exit` / `n-no-new-require`

Upstream matches by name only (an identifier named process / require), with no scope analysis. n-no-process-env's default options allow no variable. n-no-process-exit is hand-written (not generated by node_policies.py).

### `n-no-sync`

Upstream selectors `CallExpression > MemberExpression.callee Identifier[name=/Sync$/]` and `CallExpression > Identifier[name=/Sync$/]`, reported at the identifier's ESTree parent: a call with a /Sync$/ identifier as callee or argument, or a node inside (or being) a call's member callee with a /Sync$/ identifier child. So a `…Sync` identifier passed as an argument is also reported.

### `n-prefer-global-*` (buffer, console, process, text-decoder, text-encoder, url, url-search-params)

Default "always". References are found as ReferenceTracker (legacy mode) does: `require('<mod>')`/`require('node:<mod>')` with the global require, `process.getBuiltinModule(…)` with the global process, through constant aliases, member reads, object-pattern properties, import/export specifiers and `export *`. For console and process, the module itself and every ESM import/export declaration from it are reported.

### `n-prefer-node-protocol`

Flags a string naming a module in the plugin's NodeBuiltinModules table (lib/unsupported-features/node-builtins.js) that also has a `node:<name>` entry, used as the only argument of a non-optional `require(…)`, the first argument of `process.getBuiltinModule(…)`/`globalThis.process.getBuiltinModule(…)`, an import/export source, or an `import(…)` argument. The file's Node range (nearest package.json `engines.node`, else its `devEngines.runtime` entry named node, else >=16.0.0) must be a subset of ^12.20.0 || >=14.13.1 for imports and ^14.18.0 || >=16.0.0 for require; process.getBuiltinModule is checked regardless. Not ported: the `version` option and the `n`/`node` settings that override the range.

### `n-prefer-promises-dns`

Callback API covered: lookup, lookupService, getServers, resolve, resolve4, resolve6, resolveAny, resolveCname, resolveMx, resolveNaptr, resolveNs, resolvePtr, resolveSoa, resolveSrv, resolveTxt, reverse, setServers, and construction of Resolver. Found as ReferenceTracker (legacy mode) does, via require, process.getBuiltinModule, ESM default/namespace imports, constant aliases, destructuring and import specifiers. Unlike ReferenceTracker it does not follow assignments to imported or const bindings (those throw at runtime).

### `n-prefer-promises-fs`

Callback API covered: access, copyFile, open, rename, truncate, rmdir, mkdir, readdir, readlink, symlink, lstat, stat, link, unlink, chmod, lchmod, lchown, chown, utimes, realpath, mkdtemp, writeFile, appendFile, readFile, cp, glob, lutimes, opendir, rm, statfs. Same reference tracking as n-prefer-promises-dns. `watch` is intentionally not reported (fs.promises.watch is an async iterator, not a drop-in FSWatcher replacement).

### `n-hashbang`

Candidate for eslint-plugin-n 18.4.1 n/hashbang (default options). Whether a file needs a shebang is read from the nearest package.json's `bin` (a path, or an object of paths, relative to the manifest's folder, `.js` and `/index` optional); without a package.json nothing is checked, as upstream. A file `bin` does not name reports a shebang (`#!` and at least one character opening the text, ended by a line break): "Remove the shebang line; package.json does not list this file as a bin executable.". A file it names reports a first line that is no node shebang (`#!/usr/bin/env`, env flags and variables as upstream's isNodeShebang strips them, then `node`): "Add #!/usr/bin/env node as the first line; package.json lists this file as a bin executable.". Not ported: the convertPath option and settings, ignoreUnpublished (.npmignore), additionalExecutables and executableMap (the expected executable is always node), and the BOM and CRLF reports on a bin file whose shebang is right ("This file must not have Unicode BOM.", "This file must have Unix linebreaks (LF).").

### `n-no-deprecated-api`

eslint-plugin-n 18.4.1 n/no-deprecated-api (default options; the configured version only words the message). Reports every read, call or construction its deprecation trace map marks (generated from lib/rules/no-deprecated-api.js rawModules and globals), as ReferenceTracker (legacy mode) reaches it: require/process.getBuiltinModule/ESM imports of the modules (with and without `node:`), global variables (declared by globals.node, the ES builtins or `/* global */` comments) and members of global/globalThis, through constant aliases, (nested) destructuring, defaults and member chains. No sourceType is pinned, so each file keeps its own module system (.cjs/.cts CommonJS; .mjs/.mts/.tsx an ES module; .js/.jsx/.ts by the nearest package.json's type, Node's syntax detection where that package.json sets no valid type, and an ES module where there is no package.json). As eslint-plugin-n's own configs (flat/mixed-esm-and-cjs picks per file), a CommonJS file has require, module, exports, `__dirname` and `__filename` (flat/recommended-script) and an ES module none of them (flat/recommended-module turns them off). So `require` and its members are reached only in a CommonJS file; Buffer, global and process in every file.
