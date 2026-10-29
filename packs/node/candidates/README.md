# Rerail rule pack: Node.js, candidates

The ports in `rules/` here are still being evaluated: scored against the tools' expected findings and reported, never
gated and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.
Every port targets eslint-plugin-n 18.4.1 with default options and reads each file's module system as the pack's
[README](../README.md) describes.

### `n-prefer-promises-dns`

Callback API covered: lookup, lookupService, getServers, resolve, resolve4, resolve6, resolveAny, resolveCname, resolveMx, resolveNaptr, resolveNs, resolvePtr, resolveSoa, resolveSrv, resolveTxt, reverse, setServers, and construction of Resolver. Found as ReferenceTracker (legacy mode) does, via require, process.getBuiltinModule, ESM default/namespace imports, constant aliases, destructuring and import specifiers. Unlike ReferenceTracker it does not follow assignments to imported or const bindings (those throw at runtime). `require(…)` is reached only in CommonJS files; in ES modules, only imports and `process.getBuiltinModule(…)` are.

### `n-hashbang`

Candidate for eslint-plugin-n 18.4.1 n/hashbang (default options). Whether a file needs a shebang is read from the nearest package.json's `bin` (a path, or an object of paths, relative to the manifest's folder, `.js` and `/index` optional); without a package.json nothing is checked, as upstream. A file `bin` does not name reports a shebang (`#!` and at least one character opening the text, ended by a line break): "Remove the shebang line; package.json does not list this file as a bin executable.". A file it names reports a first line that is no node shebang (`#!/usr/bin/env`, env flags and variables as upstream's isNodeShebang strips them, then `node`): "Add #!/usr/bin/env node as the first line; package.json lists this file as a bin executable.". Not ported: the convertPath option and settings, ignoreUnpublished (.npmignore), additionalExecutables and executableMap (the expected executable is always node), and the BOM and CRLF reports on a bin file whose shebang is right ("This file must not have Unicode BOM.", "This file must have Unix linebreaks (LF).").

### `n-no-deprecated-api`

eslint-plugin-n 18.4.1 n/no-deprecated-api (default options; the configured version only words the message). Reports every read, call or construction its deprecation trace map marks (generated from lib/rules/no-deprecated-api.js rawModules and globals), as ReferenceTracker (legacy mode) reaches it: require/process.getBuiltinModule/ESM imports of the modules (with and without `node:`), global variables (declared by globals.node, the ES builtins or `/* global */` comments) and members of global/globalThis, through constant aliases, (nested) destructuring, defaults and member chains. No sourceType is pinned, so each file keeps its own module system (.cjs/.cts CommonJS; .mjs/.mts/.tsx an ES module; .js/.jsx/.ts by the nearest package.json's type, Node's syntax detection where that package.json sets no valid type, and an ES module where there is no package.json). As eslint-plugin-n's own configs (flat/mixed-esm-and-cjs picks per file), a CommonJS file has require, module, exports, `__dirname` and `__filename` (flat/recommended-script) and an ES module none of them (flat/recommended-module turns them off). So `require` and its members are reached only in a CommonJS file; Buffer, global and process in every file.
