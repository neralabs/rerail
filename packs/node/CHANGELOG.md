# @neralabs/rerail-pack-node

Every notable change to this rule pack, latest first. Each release says how many rules the pack ships, then lists its changes, rule by rule, under Added, Changed, Deprecated, Removed, Fixed and Security, as in [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/); releases from before the pack's notes took that format list them without those headings. There are two differences: a heading reads `## <version> (<date>)` rather than `## [<version>] - <date>`, and changes waiting for the next release are [changesets](../../.changeset/README.md), so there is no `## Unreleased`.

Versions are numbered as in [Semantic Versioning](https://semver.org/spec/v2.0.0.html), which promises nothing about compatibility below 1.0.0. Below 1.0.0 every release is a patch, and a patch can add, change or remove rules, so read the notes before you update.

## 0.1.7 (2026-11-06)

37 rules, 11 of them on by default.

### Changed

- The pack's npm page lists keywords: Rerail, its language and the tools it ports, so a search on npm finds it. Its rules are unchanged.

## 0.1.6 (2026-11-04)

37 rules, 11 of them on by default.

- 37 new rules from eslint-plugin-n: 11 start in Steer and 26 off.
- Set-up now recommends the pack for a repository with a Node.js server framework (`express`, `koa`, `fastify`, `@hapi/hapi`, `@nestjs/core`, `restify`, `@adonisjs/core`, `sails`, `@feathersjs/feathers`, `moleculer`) or `electron` dependency, or with `.cjs`, `.mjs`, `.cts` or `.mts` files outside tests, scripts, fixtures and configuration. An `@types/node` dependency alone, which browser projects carry for tooling, does not.

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
