# Rerail rule pack: Node.js

42 ports of eslint-plugin-n 18.4.1 rules with its default options. A port is in `rules/` and `pack.json` when it passes
the pack's conformance check, and in `candidates/rules/` until then ([docs/port-readiness.md](../../docs/port-readiness.md),
[conformance/README.md](../../conformance/README.md)). Each port is scored through the engine's production workspace
path, with every disagreement declared in `divergences.json` (`n-prefer-promises-fs` does not report `fs.watch`;
`n-callback-return` does not report a final `callback?.()`; `n-no-unsupported-features-es-syntax` cannot read a tagged
template with an invalid escape), against:

- **the plugin's own findings on its rule tests** (`fixtures`);
- **the tool-pack's real-code samples** (`samples.lock`, `SAMPLES.md`): files from fastify and prettier labelled with
  the plugin's findings, and the Node test set: 323 real files from 49 MIT and Apache-2.0 repositories, each at the
  commit that introduced a violation of one rule (about ten per rule), beside its nearest package.json, labelled with
  the plugin's findings for that rule, and 279 copies with those findings fixed by hand, clean for it.

33 ports ship, 7 of them on (the ones the plugin's recommended configurations turn on: `n-hashbang`,
`n-no-deprecated-api`, `n-no-exports-assign`, `n-no-process-exit`, `n-no-unsupported-features-es-builtins`,
`n-no-unsupported-features-es-syntax`, `n-no-unsupported-features-node-builtins`) and the other 26 off until a
repository turns them on. The 9 candidates: `n-no-restricted-import` and `n-no-restricted-require` report nothing at
upstream's default options, so neither source shows them catching anything; the seven rules that read where a
specifier resolves (`n-no-missing-import`/`-require`, `n-file-extension-in-import`, `n-no-unpublished-import`/`-require`,
`n-no-extraneous-import`/`-require`) pass the plugin's tests, and no sample judges them yet: a sample carries its file
and its package.json, not the files and installed packages its imports resolve to.

It follows the [pack structure and validation contract](../../docs/pack-format.md) and is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack node"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): the shipped rules, and the match that recommends it for a repository with a Node.js server framework or Electron dependency, or `.cjs`/`.mjs`/`.cts`/`.mts` files outside tests, scripts, fixtures and configuration |
| `rules/` | The shipped ports; `node_policies.py` writes every document (its `SHIPPED`) but the hand-written `n-no-process-exit` |
| `conformance.json` | Each ported rule's upstream rule, how findings compare, and the capabilities it exercises; the pinned plugin |
| `divergences.json` | Declared disagreements with the plugin |
| `upstream/eslint-plugin-n/` | The plugin's copied rule tests (`tests/`), one file per test case (`fixtures/`), its recording (`fixtures.expected.json`), the recorder (`generate.mjs`, sharing `conformance/recorders/eslint-record.mjs`), and the lock naming its real-code samples (`samples.lock`, fetched by `npm run fixtures:pull`) with their sources and licences (`SAMPLES.md`) |
| `candidates/` | The ports that do not pass yet (`rules/`, listed in `conformance.json`) and the Node probes (`probes/`), scored, never shipped |
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
no-mixed-requires, prefer-node-protocol) report in both. The recorder lints each fixture the same way:
`globals.node`, without those five names in an ES module. Upstream's RuleTester declares `globals.node` in every case,
so a case it runs as a module that reads one of those names and has no module syntax is written as CommonJS (`.js`
under the fixtures' `"type": "commonjs"` package.json), where Node defines them. A case configured by nothing but a
Node version (its `version` option, or the `n.version` setting) is written beside a package.json whose `engines.node`
is that version, which the plugin reads when no option sets one, so the rules that read the configured range
(`n-no-unsupported-features-*`, `n-prefer-node-protocol`, `n-prefer-process-get-builtin-module`) are judged at the
range each case states.

The rules that read where a file sits in its package do it through `file.package` conditions on its nearest
package.json: `bin` (its `bin` targets, as the plugin's `isBinFile` resolves them), `nodeVersion` (its configured Node
range), `private` (its `private` is `true`) and `published` (npm publishes the file, as the plugin's `getNpmignore`
reads the manifest's `files` and `main` and the `.npmignore`, else, without `files`, the `.gitignore` in its folder;
[docs/code-rule-dsl-reference.html](../../docs/code-rule-dsl-reference.html)). Without a package.json none of them
holds, `published: false` included, as the plugin checks nothing there.

The rules that read where an import or `require()` target resolves (`n-no-missing-*`, `n-file-extension-in-import`,
`n-no-unpublished-import`/`-require`, `n-no-extraneous-*`) do it through `importTarget` conditions on the specifier:
the engine resolves it as the plugin's `ImportTarget` does (enhanced-resolve with the plugin's options: `exports`
and `imports` conditions, `node_modules`, main fields and files, the tried extensions, a TypeScript file's tsconfig
`paths` and extension mapping), over the files the workspace holds and, below the workspace root, the files on disk,
so a bare specifier resolves only where its package is installed in the repository. It then reads the facts the
plugin's checks read: `failed` (no file found), `extensionSpelled`, `published` (npm publishes the target file, read
as `file.package.published` is), `devDependency`, `declared` (a dependency field of the importing file's package.json,
or of the workspace root that claims it) and `aliased` (a tsconfig `paths` key). The plugin finds packages in any
`node_modules` up to `/`; the engine looks no higher than the workspace root, and reads letter case as the disk does.

The recorder writes each case into a copy of the whole upstream fixture folder it runs in (its package.json,
`.npmignore`, `.gitignore`, `node_modules` and sibling files), lints that copy in a temporary folder so nothing of this
repository is found, and the scoring workspace's root is the fixtures folder, so resolution reads the same tree. A
case upstream runs as a module at a `.js`/`.ts` fixture path beside a package.json without `"type": "module"` is
written as `.mjs`/`.mts` in the same folder, so its package and module system both hold.

Re-recording is a maintainer deriving the pack's test data from its pins and archiving it
([CONTRIBUTING.md](../../CONTRIBUTING.md#pack-test-data)); it is never committed. The recipe
(`scripts/pack-data/recipes/node.ts`) runs `upstream/eslint-plugin-n/generate.mjs` against the pinned tools (eslint
10.11.0, eslint-plugin-n 18.4.1, @typescript-eslint/parser 8.70.1, typescript 5.9.3, globals 17.13.0).
Regenerating the rule documents runs offline, against the same tools installed under `.rerail/tools/node`:

Paused: `fixtures:derive` refuses every tool-pack until the generators write per-repo provenance
([docs/upstream.md](../../docs/upstream.md#derive-and-bumps-are-paused)). The derive and stage commands below take a
snapshot once the pause lifts.

```sh
npm run fixtures:derive -- --pack node
npm run fixtures:push -- --pack node     # on your own branch: archive it and rewrite its test-data.lock
python3 scripts/generate/packs/node_policies.py
```

Scoring one rule with every false positive and false negative printed, on the tests and on the samples (`npm run
fixtures:pull` fetches them), and the probes:

```sh
node --import tsx conformance/languages/cli.ts --pack node --rule n-no-process-exit --fixtures
node --import tsx conformance/languages/cli.ts --pack node --rule n-no-process-exit --samples
node --import tsx conformance/languages/cli.ts --pack-dir packs/node/candidates/probes --fixtures
```

The Node test set's files were mined outside this repository, from full clones of the source repositories: a history
scan, then a pattern search for the rules the scan left short, keeping only violations a commit introduced, with every
fixed copy checked to parse, to have no finding of its rule, to add no suppression comment and to keep 70% of its
lines. They are samples like any other: a new set is a new archive and `samples.lock` ([docs/port-readiness.md](../../docs/port-readiness.md#samples)).

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

### `n-prefer-global-*` (buffer, console, crypto, process, text-decoder, text-encoder, timers, url, url-search-params)

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

### `n-no-unpublished-bin`

eslint-plugin-n 18.4.1 n/no-unpublished-bin (default options). Reports, at the start of the file, a file the nearest package.json's `bin` names (read as `n-hashbang` reads it) that npm leaves out of the package (`file.package.published: false`, upstream's get-npmignore.js: the `files` whitelist, the package folder's `.npmignore`, or its `.gitignore` when there is neither a `files` field nor a `.npmignore`; never `main` or `package.json`). Without a package.json nothing is checked. The message cannot name the file's path relative to the package, which upstream's does. Not ported: the `convertPath` option and setting.

### `n-no-top-level-await`

eslint-plugin-n 18.4.1 n/no-top-level-await (default options: `ignoreBin` false). Reports each `await` expression, `for await (…)` and `await using` declaration outside every function (ESLint's `:function`: function declarations and expressions, arrows, and a method's parameters and body; its key and decorators, class fields and static blocks are outside), in a file npm publishes (`file.package.published: true`): a file without a package.json, or one the package's `files`, `.npmignore` or `.gitignore` leaves out, is not checked. Not ported: the `ignoreBin` and `convertPath` options and the `convertPath` setting.

### `n-prefer-import-assert-strict`

eslint-plugin-n 18.4.1 n/prefer-import/assert-strict. Reports the string literal `"node:assert"` (exactly: no template, no bare `"assert"`) as the source of an import, export or `import()`, or the first argument of a call of any function named `require`, unless only `strict` is taken from it: import or export specifiers that all name `strict`; a require read as `.strict`/`["strict"]` (through parentheses), destructured into properties that are all `strict` (a string key is not), or assigned to a `.strict` member.

### `n-prefer-process-get-builtin-module`

eslint-plugin-n 18.4.1 n/prefer-process-get-builtin-module, where the configured Node range is within `^20.16.0 || >=22.3.0`. Reports `require(M)` (one argument, not optional; `require` unshadowed, or a `const` made by `createRequire` imported from `module`/`node:module`) and `await import(M)` (no options), M a constant naming a built-in module (Node's `isBuiltin`: the plugin's list with and without `node:`, and `module.builtinModules` of the Node.js running the generator), unless a binding named `process` is in scope. Process shadowing is read structurally: a declaration in an enclosing block or the program, a `var` anywhere in an enclosing function, a parameter of an enclosing function or catch clause.

### `n-no-unsupported-features-node-builtins` / `n-no-unsupported-features-es-builtins`

eslint-plugin-n 18.4.1 n/no-unsupported-features/node-builtins and es-builtins (default options: no `ignores`, `allowExperimental` off). Every reference eslint-utils' ReferenceTracker finds in legacy mode (require, `process.getBuiltinModule`, import and export, globals, `import.meta` for node-builtins) to an entry of the rule's trace map whose `supported` versions do not cover the configured Node range (the range is a subset of `^v` for each supported version or `>=` the latest), one clause per supported-version list, gated on `file.package.nodeVersion.notSubsetOf`; an entry with no supported version is reported everywhere. node-builtins' map lists each module with and without `node:`, whose own versions differ: the members are traced once for both names and each name's module read apart. A global is traced only where globals.node declares it. node-builtins is 933 clauses (about 2.4 MB), which takes the engine about 3 s to compile.

### `n-no-unsupported-features-es-syntax`

eslint-plugin-n 18.4.1 n/no-unsupported-features/es-syntax (default options: no `ignores`). One clause per eslint-plugin-es-x 7.8.0 rule es-syntax.json lists (199 features; their AST rules in `scripts/generate/packs/node_es_syntax/`), gated on the configured Node range not being a subset of the feature's supported range (`<0` or none: always); a feature with a `strictMode` range (block-scoped functions and variables, classes) runs where the range is not within that range and reports in strict code (a class, a `"use strict"` prologue, a module) only where it is not within the supported range either. Utilities several clauses carry verbatim are shared through one define. Against ESLint the port reports nothing ESLint does not on es-x's own tests, about 1,850 edge cases, 2,829 real JavaScript files and 181 real TypeScript files; each module's `LIMITS` names what it misses: files tree-sitter cannot parse (a tagged template with an invalid escape), escapes after an identifier's start, keys only constant folding resolves, and, in TypeScript, the prototype-method features (`Array.prototype.includes` and the like), whose receivers es-x types with the TypeScript checker.

### `n-no-restricted-import` / `n-no-restricted-require`

eslint-plugin-n 18.4.1 n/no-restricted-import and no-restricted-require, whose default restricts nothing. The targets are the plugin's: an import or export declaration's source, an `import()` string, and the first argument of the global `require` or `require.resolve` (any constant it folds to). The restrictions are the port's `restricted` option, regular expressions each matching a whole module name (the specifier up to its first `!`), where eslint-plugin-n takes globs and a message per entry; without any, nothing is reported.

### `n-no-missing-import` / `n-no-missing-require`

eslint-plugin-n 18.4.1 n/no-missing-import and n/no-missing-require (default options). Reports each target of the plugin's visitImport (an import or export declaration's source, the literal argument of `import()`: a string, number, boolean, null or regular expression, read as its `String()`) or visitRequire (the first argument of a call of the global `require` or `require.resolve`, whatever constant it folds to) that is no Node.js built-in module and whose resolution fails (`importTarget: { failed: true }`). The message cannot repeat the resolver's error, which upstream's does. Not ported: the `allowModules`, `resolvePaths`, `resolverConfig`, `tryExtensions`, `tsconfigPath`, `typescriptExtensionMap` and `ignoreTypeImport` options and their settings.

### `n-file-extension-in-import`

eslint-plugin-n 18.4.1 n/file-extension-in-import (default style `always`, no overrides). Reports a relative or absolute import target that resolves to a file whose expected extension the specifier does not end with: the file's own, a TypeScript file's JavaScript one (`.ts` → `.js`, `.mts` → `.mjs`, …), or, for a folder, its `index` file's (`./dir` expects `./dir/index.js`). A target that resolves to nothing is the other rules' business. No fix is offered. Not ported: the `never` style, per-extension overrides, `tryExtensions` and the `typescriptExtensionMap` option and setting.

### `n-no-unpublished-import` / `n-no-unpublished-require`

eslint-plugin-n 18.4.1 n/no-unpublished-import and n/no-unpublished-require (default options: `ignorePrivate` true). In a file npm publishes (`file.package.published: true`) of a package that is not private (`private: false`), reports a path target npm does not publish (`importTarget: { published: false }`: outside the package, or left out by its `files`, `.npmignore` or `.gitignore`) and a package target only `devDependencies` lists (`devDependency: true`). A target that resolves to no file is read at the workspace root, which stands for the plugin's working directory. Not ported: the `allowModules`, `convertPath`, `resolvePaths`, `resolverConfig`, `tryExtensions` and `ignoreTypeImport` options and their settings.

### `n-no-extraneous-import` / `n-no-extraneous-require`

eslint-plugin-n 18.4.1 n/no-extraneous-import and n/no-extraneous-require (default options). Reports a package target that resolves (`resolved: true`) but that neither the importing file's package.json (its own name and its `dependencies`, `devDependencies`, `peerDependencies` and `optionalDependencies`) nor the workspace root that claims it (`workspaces`, or `pnpm-workspace.yaml`) lists (`declared: false`; a type import also counts its `@types` package), and that no tsconfig `paths` key names (`aliased: false`). Without a package.json nothing is checked. Not ported: the `allowModules`, `convertPath`, `resolvePaths`, `resolverConfig` and `tryExtensions` options and their settings.

## Not ported

- `n/process-exit-as-throw`: it reports nothing; it marks `process.exit()` as the end of a code path for ESLint's other rules, which Rerail's queries do not share.
- The deprecated `n/no-hide-core-modules` and `n/shebang` (renamed `n/hashbang`, which is ported).

