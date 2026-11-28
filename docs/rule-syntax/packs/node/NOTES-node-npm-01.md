# Notes: batch node-npm-01 (six eslint-plugin-n rules made to report what their JSON originals report)

Rules: n-no-missing-import, n-no-missing-require, n-no-top-level-await, n-no-unpublished-bin, n-no-unpublished-import,
n-no-unpublished-require. Each was translated and `different` at the start (baseline at engine a5c03d0237); each is
`same` now on every corpus the suite compares (fixtures, rerail-typescript), run alone and all six together
(`ptcheck --files … -- --pack node --rules <ids>`).

Ground truth is eslint-plugin-n 18.4.1 with ESLint 9 and @typescript-eslint/parser 8 on a fleet box, default options,
run from the corpus root with the package-type module system (`.js`/`.jsx` CommonJS unless the nearest package.json
says `"type": "module"`; `.cjs` CommonJS; `.mjs` and TypeScript modules; Node's CommonJS globals declared for
CommonJS and TypeScript files). On the fixtures the original equals ESLint for all six rules (530 keys). On
rerail-typescript ESLint gives five keys neither side gives, none a rule difference: four are in files under a `build`
folder (`test/scripts/build/pull-engine.test.mjs:4,5`, `test/support/build/script-bundle.ts:3,4`), which the corpus
walk skips and the ESLint run was given by name; one (`test/engine/storage/private-files.test.ts:103`,
`require.resolve(…)` in an ES module) comes from the ESLint run declaring `require` in TypeScript files, which the
package-type system does not (the package is `"type": "module"`).

## The cause: two inputs no one gives

All six rules read two relations the libraries declare as proposed native inputs and that have no rows:
node-resolve.rerail's `spec_resolved` (the file a specifier resolves to) and node-package.rerail's `npm_ignored` (the
files npm leaves out). So every specifier failed to resolve (2529 relative imports in rerail-typescript reported
missing: `import … from "../engine/file-replace.js"` in `src/activation/customer-profile.ts`, which is
`file-replace.ts` through the plugin's `.js` → `.ts` map) and every file counted as published (48 top-level awaits and
368 unpublished imports in rerail-typescript, whose package.json `files` is `["dist", …]`, so `src/` is not published).

Deriving rows into those `.input` relations from another file is refused by the engine (E1), so the six rules now
read a new library, `lib/tsx/node-npm-01.rerail`, that computes both from the engine's file system facts (`file`,
`dir_entry`, `file_line`, `package_field`, `tsconfig_for`, `tsconfig_path`): `npm01_resolved`, `npm01_failed`,
`npm01_leaves_out`, `npm01_published`, `npm01_unpublished` (node-resolve's `spec_unpublished` over the two). Its header
says what it follows of the plugin's enhanced-resolve options and getNpmignore.

## Per rule

### n-no-missing-import (fixtures 1/6, rerail-typescript 0/2529 → same)

- Relative names resolve as the plugin's resolver does: the file, then the try extensions; in a TypeScript file the
  extension map (`./a.js` is `./a.ts` only under `jsx` preserve, and `./a.js` itself is not tried) and tsconfig
  `paths`. 2529 of the rerail-typescript keys.
- `import(0)` (`prefer-node-protocol/valid-006.mjs:3`): the plugin names the module `String(0)`, a package "0" that no
  folder has, and reports it. node-resolve's `node_target` takes string constants only, so the library adds `npm01_lit`
  (number, boolean, null and regex literals) and the rule reports it.
- `import isIp from '#is-ip'` with package.json `"imports": { "#is-ip": { "node": "net" } }`
  (`no-missing-import/valid-054`): the plugin's resolver maps it to the built-in and does not report it.
- `import '@typescript-eslint\parser'` and other names that are no package and no path still fail, as in the plugin.

### n-no-missing-require (fixtures 4/7 → same)

- `require(42)`, `require(2)`, `require(777)` (`no-mixed-requires/valid-008`, `no-restricted-require/valid-004`,
  `no-unpublished-require/valid-028`): reported, as `import(0)` above.
- `require("./x")` of a file that exists (`no-deprecated-api/valid-014.js`, `no-unsupported-features/node-builtins/
  valid-254/case.js`, …): no longer reported.

### n-no-top-level-await (fixtures 0/10, rerail-typescript 0/48 → same)

`await` at the top of `src/cli/cli.ts:850`: package.json's `files` lists `dist`, not `src`, so npm does not publish the
file and the plugin skips it. The fixtures' `simple-files` and `dot-slash-files` cases are the same with `files`
entries `src/a.mjs` and `./src/a.mjs` (a leading `./` is dropped, as the plugin's posix normalize does).

### n-no-unpublished-bin (fixtures 13/0 → same)

`no-unpublished-bin/invalid-001` (`{ "bin": "a.js", "files": ["b.js"] }`): a.js is a bin file npm does not publish;
reported now that `files`, `.npmignore` and `.gitignore` are read. `valid-013` (`"{bin,src}/**/!(*.test).js"`): the
extglob is read as below.

### n-no-unpublished-import (fixtures 9/7, rerail-typescript 0/368 → same)

- `src/hosts/omp/extension.ts:7` imports a devDependency, but `src/` is not published, so the plugin does not check
  the file. 368 keys.
- `no-unpublished-import/invalid-018/no-unpublished/private-package`: package.json says `"private": true`; with
  ignorePrivate (the default) the plugin skips it. `package_field` gives no row for a boolean (E3), so the library reads
  the package.json lines for `"private": true`.
- `#is-ip` in a published file of a nested package (`no-missing-import/valid-054`): its target has no file, so the
  plugin tests the working directory (the root) against the package folder; that is outside the package, so it
  reports it. The library's `npm01_target` gives such a name the root (`""`).

### n-no-unpublished-require (fixtures 5/14 → same)

- `brace-extglob` (`"files": ["{bin,src}/**/!(*.test).js"]`): `require('./src/helper.js')` is published
  (`valid-037`), `require('./src/helper.test.js')` is not (`invalid-001`). Core's glob has braces but no extglob:
  `!(p)` is read as `*` except the paths the glob with `p` in its place matches; `@(a|b)` as `{a,b}`; `*(…)`, `+(…)`,
  `?(…)` as `*`.
- `brace-sequence` (`"files": ["file{1..3}.js"]`, `valid-039`): one numeric sequence per entry is expanded, as the
  plugin's expandSequenceBraces does.
- `.npmignore` and `files` cases (`invalid-003`, `invalid-008`, `invalid-009`, `valid-007`, `valid-013`, …): follow
  from reading them.

## What the library does not follow (no corpus exercises it)

Each follows the plugin as far as the engine's facts allow; a counterexample for each:

- A package name resolves where `node_modules/<name>` is a folder at or above the importing file. Its subpath and
  `exports` are not read: `require("lodash/nope")` with lodash installed is not reported; the plugin reports it. In a
  TypeScript file `import "pkg/x.js"` is reported, as the plugin does, since it tries `pkg/x.ts` only.
- A package's self-reference through its own `exports` (`import "@me/pkg/x"` inside @me/pkg) fails here; the plugin
  resolves it.
- `allowImportingTsExtensions` and `jsx` are read from the nearest tsconfig.json's own lines, not its `extends` chain,
  and jsconfig.json is not read.
- Symlinks are not resolved to their targets.
- `"files": []` (no string entry, so no `package_field` row) counts as no `files` list; npm and the plugin publish
  only the always-published files then.
- Extglobs as above: `files: ["lib/+(a|b).js"]` takes `lib/c.js` too.
- The walk enters every folder but version control, node_modules, caches and `target` once per run: a repository with
  a large `dist` or `vendor` pays for listing it.

## Engine gaps

- **E1.** A rule that derives rows into a relation a library declares `.input` (`npm_ignored(f) :- …`) is refused at
  planning: `no statistics for npm_ignored.c0`. So a library cannot supply a proposed input until core does.
- **E2.** A file system request (`dir_entry`, `file_line`) whose path comes from a document's rows fails at planning:
  from a per-file relation, `no statistics for pd.c0`; from `doc`, `file`, `package_for` or `package_field`, `the cone
  of requests reads base relation package_field: a load answers exports only (decision 0065 §2a)`. Only paths built
  from constants work, which is why node-npm-01 walks the folders from the root instead of asking for each package
  folder's `.npmignore`, or for `node_modules/<name>` of each imported name.
- **E3.** `package_field(dir, "private", v)` has no row for `"private": true`, though `facts/repository.rs`'s `scalar`
  turns booleans into `"true"`; node-resolve's `pkg_private_skip` therefore never holds (NOTES-node.md D9).

## Library changes proposed

- Replace node-resolve.rerail's `.input spec_resolved` and node-package.rerail's `.input npm_ignored` with
  node-npm-01's derivations (`npm01_resolved`; `npm_ignored(f) :- pkg_file(d, dir, rel), doc(d, f, _),
  npm01_leaves_out(dir, rel)`), and `spec_failed`, `node_target_file` and `spec_unpublished` with `npm01_failed`,
  `npm01_target` and `npm01_unpublished`; then n-no-extraneous-require and n-hashbang read the same resolution and
  publication, and node-npm-01 goes. Until E1 is fixed this has to be done in those files, not beside them.
- node-resolve's `node_target`: take number, boolean, null and regex literals too (node-npm-01's `npm01_lit`), as the
  plugin's getStringIfConstant does; then the two missing rules lose their second clause.
- node-resolve's `pkg_private_skip`: read `npm01_private` until E3 is fixed.
