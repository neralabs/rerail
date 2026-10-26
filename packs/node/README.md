# Rerail rule pack: Node.js

24 ports of eslint-plugin-n 18.4.1 rules with its default options. A port is in `rules/` and `pack.json` when it passes
the pack's conformance check on both of its sources, and in `candidates/rules/` until then. eslint-plugin-n has no port
readiness gates ([docs/port-readiness.md](../../docs/port-readiness.md)); a pinned test set judges the pack instead
(`TEST_SETS` in `scripts/s3/pack-fixtures.mjs`, [conformance/README.md](../../conformance/README.md)). Each port is
scored through the engine's production workspace path, with every disagreement declared in `divergences.json`
(`n-prefer-promises-fs` does not report `fs.watch`, on both sources; `n-callback-return` does not report a final
`callback?.()`, on the test set), against:

- **the plugin's own findings on its rule tests** (`fixtures`), and on two pinned corpora (fastify, prettier);
- **the Node test set** (`node-test-set`): 239 real files from 46 MIT and Apache-2.0 repositories, each at the commit
  that introduced a violation of one rule (about ten per rule), each beside its nearest package.json, where the plugin's
  findings for that rule are expected, and the 222 copies with that rule's findings fixed by hand, where none is.

The conformance check (`--check`, run in CI) scores the tests and the test set: it fails a port in `rules/` on any
unexplained FP or FN on either, and reports the candidates without failing them. All 24 ports ship, 4 of them on (the
ones the plugin's recommended configurations turn on: `n-hashbang`, `n-no-deprecated-api`, `n-no-exports-assign`,
`n-no-process-exit`) and the other 20 off until a repository turns them on.

It follows the [pack structure and validation contract](../../docs/pack-format.md) and is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack node"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): the shipped rules, and the match that recommends it for a repository with a Node.js server framework or Electron dependency, or `.cjs`/`.mjs`/`.cts`/`.mts` files outside tests, scripts, fixtures and configuration |
| `rules/` | The shipped ports; `node_policies.py` writes every document (its `SHIPPED`) but the hand-written `n-no-process-exit` |
| `conformance.json` | Each ported rule's upstream rule, how findings compare, and the capabilities it exercises; the pinned plugin and the pinned corpora (fastify, prettier) |
| `divergences.json` | Declared disagreements with the plugin |
| `upstream/eslint-plugin-n/` | The plugin's copied rule tests (`tests/`), one file per test case (`fixtures/`), its recordings (`fixtures.expected.json`, `corpus-<name>.expected.json`) and the recorder (`generate.mjs`, sharing `conformance/recorders/eslint-record.mjs`) |
| `candidates/` | The Node probes (`probes/`), scored, never shipped; a port that stops passing goes back to `candidates/rules/` |
| `licenses/eslint-plugin-n/` | The plugin's licence and `UPSTREAM.md`, listing every copied file |

No rule pins `languageOptions.sourceType` (the probes do, to state Node facts). The rules that reach CommonJS names
through scope set `languageOptions.moduleSystem: "package-type"`, so each file has the module system the plugin's
`flat/mixed-esm-and-cjs` config gives it, by extension and package.json `type` alone: `.mjs`/`.mts` an ES module,
`.cjs`/`.cts` CommonJS, any other file an ES module only under `"type": "module"` and CommonJS otherwise (a `type`-less
package.json and no package.json included, whatever the file's syntax: tsc compiles such TypeScript to CommonJS). The
engine's own default, Node's syntax detection where `type` is unset and an ES module without a package.json, is left to
the other packs. Only a CommonJS file has `require`, `module`,
`exports`, `__dirname` and `__filename`; an ES module has none (`flat/recommended-module` turns them off), so the rules
that reach them through scope (exports-style, no-exports-assign, no-path-concat, prefer-global/\*, prefer-promises/\*,
no-deprecated-api) report them in CommonJS files only. Rules upstream reads by name (global-require, no-new-require,
no-mixed-requires, prefer-node-protocol) report in both. The recorder lints each fixture and corpus file the same way:
`globals.node`, without those five names in an ES module. Upstream's RuleTester declares `globals.node` in every case,
so a case it runs as a module that reads one of those names and has no module syntax is written as CommonJS (`.js`
under the fixtures' `"type": "commonjs"` package.json), where Node defines them.

Re-recording is a maintainer deriving the pack's test data from its pins and archiving it
([CONTRIBUTING.md](../../CONTRIBUTING.md#pack-test-data)); it is never committed. The recipe
(`scripts/pack-data/recipes/node.ts`) runs `upstream/eslint-plugin-n/generate.mjs` against the pinned tools (eslint
10.11.0, eslint-plugin-n 18.4.1, @typescript-eslint/parser 8.70.1, typescript 5.9.3, globals 17.13.0) and corpora.
Regenerating the rule documents runs offline, against the same tools installed under `.rerail/tools/node`:

Paused: `fixtures:derive` refuses every tool-pack until the generators write per-repo provenance
([docs/upstream.md](../../docs/upstream.md#derive-and-bumps-are-paused)). The derive and stage commands below take a
snapshot once the pause lifts.

```sh
npm run fixtures:derive -- --pack node
npm run fixtures:push -- --pack node     # on your own branch: archive it and rewrite its test-data.lock
python3 scripts/generate/packs/node_policies.py
```

Scoring one rule with every false positive and false negative printed, on the tests and on the test set (fetched from
S3 and verified the first time), and the probes:

```sh
node --import tsx conformance/languages/cli.ts --pack node --rule n-no-process-exit --fixtures
node --import tsx conformance/languages/cli.ts --pack node --rule n-no-process-exit --test-set
node --import tsx conformance/languages/cli.ts --pack-dir packs/node/candidates/probes --fixtures
```

The test set is built outside this repository, from full clones of the source repositories: a history scan, then a
pattern search for the rules the scan left short, keeping only violations a commit introduced, with every fixed copy
checked to parse, to have no finding of its rule, to add no suppression comment and to keep 70% of its lines. A new
build is a new zip in S3 and a new `TEST_SETS` pin ([CONTRIBUTING.md](../../CONTRIBUTING.md#pack-test-data)).

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### Module system per file (shared by n-exports-style, n-global-require, n-no-exports-assign, n-no-path-concat, n-prefer-global-*, n-prefer-promises-*)

All ports target eslint-plugin-n 18.4.1 with default options. No sourceType is pinned; `moduleSystem: "package-type"` gives each file the plugin's module system: .mjs/.mts are ES modules, .cjs/.cts CommonJS, any other file (.tsx included) an ES module only under package.json `"type": "module"` and CommonJS otherwise, without a package.json too. Like the plugin's flat/mixed-esm-and-cjs config, a CommonJS file has the globals require, module, exports, __dirname and __filename (flat/recommended-script) and an ES module has none (flat/recommended-module). Consequences: the global `exports` exists only in CommonJS files; `__dirname`/`__filename` path concatenation is caught only in CommonJS (the `-commonjs` clauses), while ES modules are checked for `import.meta` paths; for prefer-global and prefer-promises rules, `require(…)` is reached only in CommonJS files, and in ES modules only imports and `process.getBuiltinModule(…)` are.

### `n-callback-return`

Default callback names: callback, cb, next. A call of one of those is allowed when its nearest block/return/arrow ancestor is a return or an arrow function, when it is the last statement of a function body (or the right side of a binary/logical expression statement there), or when it is the statement just before a block's final return. Calls with a template-string argument are not matched. An optional call (`callback?.()`) gets the same exemptions; upstream reports one in those positions, because ESTree's ChainExpression around it defeats its `parentNode.expression === node` check (declared in `divergences.json`).

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

eslint-plugin-n 18.4.1 n/hashbang (default options). Whether a file needs a shebang is read from the nearest package.json's `bin` (a path, or an object of paths, relative to the manifest's folder, `.js` and `/index` optional); without a package.json nothing is checked, as upstream. A file `bin` does not name reports a shebang (`#!` and at least one character opening the text, ended by a line break): "Remove the shebang line; package.json does not list this file as a bin executable.". A file it names reports a first line that is no node shebang (`#!/usr/bin/env`, env flags and variables as upstream's isNodeShebang strips them, then `node`): "Add #!/usr/bin/env node as the first line; package.json lists this file as a bin executable.". A file it names whose node shebang is right is reported, at the start of the file, once for a byte order mark before it (`file.byteOrderMark`, since no node's text holds the mark): "Remove the byte order mark before the shebang; …", and once for a shebang line ending in CR LF: "End the shebang line with LF, not CRLF; …", as upstream reports `unexpectedBOM` and `expectedLF`. Not ported: the convertPath option and settings, ignoreUnpublished (.npmignore), additionalExecutables and executableMap (the expected executable is always node).

### `n-no-deprecated-api`

eslint-plugin-n 18.4.1 n/no-deprecated-api (default options; the configured version only words the message). Reports every read, call or construction its deprecation trace map marks (generated from lib/rules/no-deprecated-api.js rawModules and globals), as ReferenceTracker (legacy mode) reaches it: require/process.getBuiltinModule/ESM imports of the modules (with and without `node:`), and `export * from` a module once for each deprecated member it re-exports (one clause per member, as ReferenceTracker reports it), global variables (declared by globals.node, the ES builtins or `/* global */` comments) and members of global/globalThis, through constant aliases, (nested) destructuring, defaults and member chains. No sourceType is pinned; `moduleSystem: "package-type"` gives each file the plugin's module system (.mjs/.mts an ES module, .cjs/.cts CommonJS, any other file an ES module only under package.json `"type": "module"`). As eslint-plugin-n's own configs (flat/mixed-esm-and-cjs picks per file), a CommonJS file has require, module, exports, `__dirname` and `__filename` (flat/recommended-script) and an ES module none of them (flat/recommended-module turns them off). So `require` and its members are reached only in a CommonJS file; Buffer, global and process in every file.
