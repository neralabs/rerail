# @neralabs/rerail-pack-node

## 0.1.6

### Patch Changes

- 1e7042b: Node pack: the conformance check now gates its eslint-plugin-n ports, which have an upstream manifest (`upstream/eslint-plugin-n/gates.yaml`) like the other ESLint-family tool-packs. The pack still ships no rules: every port stays a candidate until it is promoted.
- cbd9380: Node.js pack: ships 11 more eslint-plugin-n rules, 32 in all, judged on the plugin's own tests and on real-code samples, among them 323 real files from open-source history at the commit that introduced each violation, labelled with eslint-plugin-n's own findings, and 279 copies fixed by hand. Six start on, as the plugin's recommended configurations turn them on: `n-hashbang`, `n-no-deprecated-api`, `n-no-exports-assign`, `n-no-process-exit`, `n-no-unsupported-features-es-builtins` and `n-no-unsupported-features-node-builtins`; the other 26 start off.
  
  New ports since the last release: `n-no-unsupported-features-node-builtins` and `-es-builtins` (a Node.js API or ECMAScript built-in that a Node.js version in the package's `engines.node` range lacks), `n-prefer-global-crypto`, `n-prefer-global-timers`, `n-prefer-import-assert-strict`, `n-prefer-process-get-builtin-module`, `n-no-top-level-await` (in a file npm publishes) and `n-no-unpublished-bin` (a bin file npm leaves out of the package, read from package.json `files`, `.npmignore` and `.gitignore` as the plugin reads them). `n-no-unsupported-features-es-syntax` (ECMAScript syntax the range lacks) is a candidate, matching the plugin but not yet compact enough to ship; `n-no-restricted-import` and `n-no-restricted-require` are candidates too: upstream restricts nothing by default, and the port's `restricted` option takes regular expressions.
  
  `n-hashbang` now also reports a byte order mark or a CR LF line ending on a bin file's shebang, `n-no-deprecated-api` reports `export * from` a module once for each deprecated member it re-exports, and the rules that reach CommonJS names through scope read each file's module system as the plugin's configs do, by extension and package.json `type` alone (a `type`-less package.json, or none, is CommonJS, whatever the file's syntax).
  
  The pack is now recommended for a repository with a Node.js server framework (`express`, `koa`, `fastify`, `@hapi/hapi`, `@nestjs/core`, `restify`, `@adonisjs/core`, `sails`, `@feathersjs/feathers`, `moleculer`) or `electron` dependency, or with `.cjs`, `.mjs`, `.cts` or `.mts` files outside tests, scripts, fixtures and configuration. An `@types/node` dependency, which browser projects carry for their tooling, no longer recommends it.
  
  Seven more eslint-plugin-n rules are ported as candidates, scored on the plugin's tests but not shipped yet: `n-no-missing-import` and `n-no-missing-require` (a module that resolves to no installed package or file), `n-file-extension-in-import` (a relative import that leaves out the extension of the file it names), `n-no-unpublished-import` and `n-no-unpublished-require` (a file npm publishes loading a file npm leaves out, or a package only `devDependencies` lists) and `n-no-extraneous-import` and `n-no-extraneous-require` (a package package.json does not list). Each resolves the specifier as the plugin does, over the repository's files and the packages installed below its root.
- e1b9a49: Node.js pack: ships its first 21 rules, now that they pass every conformance check. Start on: `n-no-exports-assign` and `n-no-process-exit`. Start off, as no approved upstream configuration turns them on: `n-callback-return`, `n-exports-style`, `n-global-require`, `n-handle-callback-err`, `n-no-callback-literal`, `n-no-mixed-requires`, `n-no-new-require`, `n-no-path-concat`, `n-no-process-env`, `n-no-sync`, `n-prefer-global-buffer`, `n-prefer-global-console`, `n-prefer-global-process`, `n-prefer-global-text-decoder`, `n-prefer-global-text-encoder`, `n-prefer-global-url-search-params`, `n-prefer-global-url`, `n-prefer-node-protocol` and `n-prefer-promises-fs`.
- 61cda45: Each pack's `covers`, the line the dashboard shows under the pack's name, says in one sentence what the pack's rules are for and where they come from. It no longer counts rules or says how many start on: the dashboard shows both beside it, and the counts went stale whenever a rule shipped.
- 1e7042b: README: the note that deriving pack test data is paused is gone, since derive runs again; no rule changes.
- c8352a7: README, `pack.json` descriptions and licence notes: the conformance check is the one gate a port passes to ship, the ECMAScript packs' tests carry rename variants that upstream labels (`holds-under-rename`), and new test data is published through a tool-pack's `publish/` directory; no rule changes.
- c8352a7: Licence notes: each `licenses/<project>/UPSTREAM.md` records its upstream repository per repo (source, the path patterns taken, what is not copied) instead of one row per copied test file; no rule changes.
- 3fb36af: The README describes only the rules the pack ships. What it said about the candidate ports moves to `candidates/README.md`, which is not published.
- c8352a7: README and licence notes: the ports are scored on labelled real-code samples instead of pinned corpora; no rule changes.

## 0.1.5

### Patch Changes

- 7392aea: README and licence notes: the pack's test data now lives outside the package; no rule changes.
- d0dd309: README: internal ticket references removed; no rule changes.

## 0.1.4

### Patch Changes

- e06db22: Node pack: messages whose rewrite only changed the voice are back to their original wording; in an eval with a coding agent, the original and rewritten messages steered it equally well.
- e06db22: Node.js pack: every rule description now says in one plain sentence what the rule wants; the exact matching detail moved to the pack README.
- e06db22: Node.js pack: every violation message now says in one sentence what to change in the flagged code, without tool names or rule codes.

## 0.1.3

### Patch Changes

- d1e67d7: Node.js pack: ships no rules until its ports pass the port readiness gates. The 22 eslint-plugin-n 18.4.1 ports move to `candidates/rules/`, still scored against the plugin's own findings on its tests and corpora, and the pack is never recommended while it has no rules.
- ac01449: Node.js pack: the rules that read CommonJS's `require`, `module`, `exports`, `__dirname` and `__filename` (`n/exports-style`, `n/no-exports-assign`, `n/no-path-concat`, the `n/prefer-global/*` and `n/prefer-promises/*` rules, and the `n/no-deprecated-api` candidate) no longer treat every file as CommonJS. Each file keeps its own module system (`.cjs`/`.cts` CommonJS; `.mjs`/`.mts`/`.tsx` an ES module; `.js`/`.jsx`/`.ts` by the nearest `package.json`'s `type`, Node's syntax detection where that package.json sets no valid `type`, and an ES module where there is no package.json), as eslint-plugin-n's `flat/mixed-esm-and-cjs` config does, so an ES module is no longer told about `exports`, `require('fs')` or `__dirname`, which Node does not define there. `n/global-require` still reports a nested `require()` in an ES module, as upstream does.
- ac01449: Node.js pack: `n/prefer-node-protocol` reads the file's Node version from its nearest `package.json` (`engines.node`, else the `devEngines.runtime` entry named `node`, else `>=16.0.0`), as eslint-plugin-n does, so a package whose Node range includes versions without `node:` support (`>=14`) is no longer told to use `node:` in `require` or imports; `process.getBuiltinModule` is checked whatever the range. The `n/hashbang` candidate reads the nearest `package.json`'s `bin`: a file `bin` names must start with a node shebang, and only other files are told they need none.
- 3049ced: The match definition names its npm dependencies in the new form, `{ "dependencies": { "ecosystem": "npm", "names": [...] } }`, which core reads alongside PyPI and Composer dependencies. Which repositories the pack fits is unchanged.

## 0.1.2

### Patch Changes

- 00983ec: Node.js pack: `n-prefer-promises-fs` and `n-prefer-promises-dns` finish on large files. Their aliasing branches read every occurrence of a callee's variable once per call, which was quadratic on large test suites (both rules reported nothing for a 14,000-line test file). A call is now read only in a file that names the module, and the aliasing branches skip imported and `const` bindings, which no program that runs can reassign. Their fixture and corpus scores are unchanged.

## 0.1.1

### Patch Changes

- f374b76: Node pack: `n-prefer-promises-fs` no longer reports `fs.watch` (its promise form returns an async iterator, not a drop-in replacement), and `n-no-callback-literal`'s message now says to check the callback is error-first before passing an error or null (ws `verifyClient` and ssh2 `authHandler` are not).
- 37c902a: Node.js pack: the 10 rules Oxlint 1.88.0 also implements honour its suppression comments naming them (`// oxlint-disable-line no-process-env` and the other `oxlint-disable` forms), as well as ESLint's. `n-no-process-env` honours Biome 2.5.15's `biome-ignore` comments naming `lint/style/noProcessEnv` (or its group, or `lint`).
- fbb6104: Node.js pack: 22 eslint-plugin-n rules, moved to packs/node with its conformance recordings, candidates and licence record.
- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
