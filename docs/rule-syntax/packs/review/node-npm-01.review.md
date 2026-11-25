# Review: node-npm-01 (eslint-plugin-n 18.4.1)

Reviewed: `tbelanger/pack-translations-node-npm-01` at fa220123f0 (the six `.rerail` files and
`lib/tsx/node-npm-01.rerail`), against eslint-plugin-n 18.4.1's sources (`lib/rules/*`, `lib/util/import-target.js`,
`get-npmignore.js`, `check-publish.js`, `is-bin-file.js`, `get-typescript-extension-map.js`, `get-try-extensions.js`).

How the tool was run: on pt-w01, in /tmp, with the node pack generator's pins and configuration
(`packs/node/upstream/eslint-plugin-n/generate.mjs`): eslint 10.11.0, eslint-plugin-n 18.4.1, @typescript-eslint/parser
8.70.1, typescript 5.9.3, globals 17.13.0; source type from the extension, else the nearest package.json's `type`;
`globals.node`, without the CommonJS wrapper names in ES modules; default options; all six rules on. ESLint's working
directory (and `process.cwd()`) is the case's root, as when a user runs ESLint from the repository root. The translation:
`~/pt/bin/rerail-engine run-policies <case> <out> <the six .rerail files>`, from a copy of this commit's
`docs/rule-syntax`. Each case below is a directory; files are given as `path: content`. Output lines are
`path:line[:col] rule`.

Besides the cases below, a differential run over the publish view: 84 packages, each with an `.npmignore`, a
`.gitignore`, a `files` list or `files` beside one of them, and the same 17 files (`await 0;` in each, so
n/no-top-level-await reports exactly the published ones). All 44 `.npmignore`/`.gitignore`-only packages agree (`test`,
`/test`, `test/`, `**/test`, `test/**`, `*` with `!lib`, `!lib/`, `!lib/**`, `!*/`, `[lt]*/`, `?ib`, `**`, `**/`, trailing
`\`, `\!`, case, …); of the 40 with `files`, four differ (D7, D8, D12).

## Summary

| Rule | Verdict |
|---|---|
| n-no-missing-import | defects D1, D2, D3, D4, D5, D6 |
| n-no-missing-require | defects D1, D3, D4, D6 |
| n-no-top-level-await | defects D7, D8, D9, D12 (all in the shared publish view; its own `await` matching is ok) |
| n-no-unpublished-bin | defects D7, D8, D11 |
| n-no-unpublished-import | defects D7, D8, D9, D10, D12 |
| n-no-unpublished-require | defects D7, D8, D9, D10, D12 |

D1-D6 are in resolution (`npm01_failed`), D7-D10 and D12 in the publish view (`npm01_leaves_out`, `npm01_checked`), D11 in
`program_start`. All are in `lib/tsx/node-npm-01.rerail` or `lib/tsx/node-package.rerail`, so each reaches every rule
that reads that part. D2 and D4 are gaps the library header states; they are listed because the tool differs there.

## Resolution (n-no-missing-import, n-no-missing-require)

### D1. A commented-out `allowImportingTsExtensions` in tsconfig.json turns the extension map off (false positives)

`npm01_tsc_allow` and `npm01_tsc_react` match each raw line of tsconfig.json with a regex, so a `//` comment counts.
TypeScript 5.0-5.8's `tsc --init` writes `// "allowImportingTsExtensions": true, /* … */` into every tsconfig.json it
creates. The translation then takes the "explicit" map (no `.js` -> `.ts` alias), and every NodeNext-style import of a
TypeScript file by its `.js` name is reported. The plugin reads the tsconfig with get-tsconfig (JSONC) and uses the
preserve map. The same applies to a commented-out `"jsx": "react…"` (the "default" map, which drops `.jsx` -> `.tsx`).

Case c01b: `tsconfig.json` is exactly what `npx tsc --init` (typescript 5.8) writes; then

```
package.json:  {"name":"c01b","version":"1.0.0"}
src/a.ts:      export const a = 1;
src/index.ts:  import { a } from "./a.js";
               export { a };
```

- ESLint: no findings.
- Translation: `src/index.ts:1 n/no-missing-import`.

### D2. tsconfig `extends` is not read for `jsx` and `allowImportingTsExtensions` (false positives and misses)

The plugin's get-tsconfig follows `extends`; the translation reads only the nearest tsconfig.json's own lines. With
`jsx: "react-jsx"` in a base config, `./App.js` names `App.tsx` for the plugin (default map: `.js` -> `.ts`, `.tsx`) and
nothing for the translation (preserve: `.js` -> `.ts` only). Stated in the library header; still a difference.

Case c02:

```
package.json:         {"name":"c02","version":"1.0.0","type":"module"}
tsconfig.base.json:   {"compilerOptions":{"jsx":"react-jsx"}}
tsconfig.json:        {"extends":"./tsconfig.base.json","compilerOptions":{"module":"nodenext"}}
src/App.tsx:          export const App = () => null;
src/main.ts:          import { App } from "./App.js";
                      export { App };
```

- ESLint: no findings.
- Translation: `src/main.ts:1 n/no-missing-import`.

### D3. Files under a folder named `target`, `coverage`, `.next` or `.turbo` are never found (false positives)

`npm01_skip` drops these names at any depth from the disk walk, and the engine's `file` (the repository walk) skips them
too, so a request into such a folder never resolves. enhanced-resolve reads them like any folder. (`dist`, `build` and
`vendor` are entered, as the header says; these four are not.)

Case c03:

```
package.json:       {"name":"c03","version":"1.0.0"}
target/util.js:     module.exports = 1;
coverage/report.js: module.exports = 1;
lib/.next/x.js:     module.exports = 1;
index.js:           require("./target/util");
                    require("./coverage/report");
                    require("./lib/.next/x");
```

- ESLint: no findings.
- Translation: `index.js:1`, `index.js:2`, `index.js:3` n/no-missing-require.

The same holds for `import` (n/no-missing-import), and for n/no-unpublished-* the unresolved target then names the
written path instead of the file.

### D4. An installed package always resolves: its subpaths, `exports` and conditions are not read (misses)

`npm01_installed_for` takes any `node_modules/<name>` folder as resolution. The plugin reports a subpath the package
lacks, a subpath its `exports` does not expose, and a package whose `exports` has no condition for the style (an
import-only package loaded with `require()`, common since packages went ESM-only). Stated in the library header; still
a difference.

Case c04:

```
package.json:                       {"name":"c04","version":"1.0.0","dependencies":{"lodash":"1","expkg":"1","esmonly":"1"}}
node_modules/lodash/package.json:   {"name":"lodash","version":"1.0.0","main":"index.js"}
node_modules/lodash/index.js:       module.exports = 1;
node_modules/expkg/package.json:    {"name":"expkg","version":"1.0.0","exports":{".":"./index.js"}}
node_modules/expkg/index.js:        module.exports = 1;
node_modules/expkg/internal.js:     module.exports = 1;
node_modules/esmonly/package.json:  {"name":"esmonly","version":"1.0.0","type":"module","exports":{".":{"import":"./index.js"}}}
node_modules/esmonly/index.js:      export default 1;
index.js:                           require("lodash/nope");
                                    require("expkg/internal.js");
                                    require("esmonly");
                                    require("lodash");
```

- ESLint: `index.js:1:9` (`Can't resolve 'lodash/nope'`), `index.js:2:9` (`"./internal.js" is not exported under the
  conditions ["node","require"]`), `index.js:3:9` (`"." is not exported under the conditions ["node","require"]`), all
  n/no-missing-require.
- Translation: no findings.

### D5. package.json `imports`: subpath patterns and nested conditions are not read (false positives)

`npm01_imports_for` takes only an exact key (`imports/#x`) or one condition level (`imports/#x/<cond>`). enhanced-resolve
also takes `#utils/*` patterns and nested conditions.

Case c05a:

```
package.json:     {"name":"c05a","version":"1.0.0","type":"module","imports":{"#utils/*":"./src/utils/*.js"}}
src/utils/a.js:   export default 1;
index.js:         import a from "#utils/a";
                  export { a };
```

Case c05b:

```
package.json:     {"name":"c05b","version":"1.0.0","type":"module","imports":{"#dep":{"node":{"import":"./src/dep.js"}}}}
src/dep.js:       export default 1;
index.js:         import d from "#dep";
                  export { d };
```

- ESLint: no findings in either.
- Translation: `index.js:1 n/no-missing-import` in each.

### D6. Non-integer constant arguments are not reported (misses; low impact)

The plugin names the module `String()` of getStringIfConstant's value, which never resolves. `npm01_lit_text` takes
only decimal integers, `true`/`false`/`null` and regexes, so decimals, hex, bigint and negated numbers are missed.

Case c06:

```
package.json:  {"name":"c06","version":"1.0.0"}
index.js:      require(42);
               require(1.5);
               require(-1);
               require(0x10);
               require(1n);
               require(`./no${"pe"}`);
               import(1.5);
```

- ESLint: lines 1-6 n/no-missing-require (`'42'`, `'1.5'`, `'-1'`, `'16'`, `'1'`, `'./nope'`), line 7
  n/no-missing-import (`'1.5'`).
- Translation: lines 1 and 6 only.

## Publish view (n-no-unpublished-*, n-no-top-level-await, n-no-unpublished-bin)

### D7. `"files": []` publishes nothing for the plugin, everything for the translation

`parseWhiteList` takes any array, so an empty `files` list leaves out every file but package.json, `main` and the
README/LICENSE names. `npm01_has_files` needs at least one entry, so the translation falls back to "publish everything".

Case (fuzz package s64):

```
package.json:  {"name":"s64","version":"1.0.0","files":[]}
a.mjs, lib/a.mjs, src/sub/c.mjs, test/a.mjs, … (17 files):   await 0;
```

- ESLint: no findings (no file is published).
- Translation: n/no-top-level-await in all 17 files.

The same package makes n/no-unpublished-import/-require and n/no-unpublished-bin miss everything they report.

### D8. A negated `files` entry with a leading `/` excludes nothing

`parseWhiteList` strips one leading `!` or `/` (`"!/lib/sub"` -> `/lib/sub`) and adds it to node-ignore, which reads a
leading `/` as an anchor. `npm01_entry` strips the same character, but the body keeps its `/` and `iglob("/lib/sub", …)`
matches no path.

Case (fuzz package s54):

```
package.json:    {"name":"s54","version":"1.0.0","files":["lib","!/lib/sub"]}
lib/a.mjs, lib/sub/b.mjs, …:   await 0;
```

- ESLint: `lib/sub/b.mjs` is not published: no n/no-top-level-await there.
- Translation: `lib/sub/b.mjs:1 n/no-top-level-await`.

### D9. README/LICENSE/CHANGES/CHANGELOG/HISTORY names in a nested package

`filterNeverIgnoredFiles` tests `NEVER_IGNORED` on `path.relative(basedir, filePath)` where `filePath` is already relative
to the package: Node resolves it against `process.cwd()`. Run from the repository root, a nested package's `history.mjs`
becomes `../../history.mjs`, so the exemption only holds in the package at the working directory. `npm01_never`
applies it in every package. The plugin's result depends on the working directory; the case below is the usual run
(ESLint at the repository root).

Case c07:

```
package.json:                  {"name":"root","version":"1.0.0","private":true}
packages/p/package.json:       {"name":"p","version":"1.0.0","files":["lib"]}
packages/p/history.mjs:        await 0;
packages/p/lib/index.mjs:      import "../history.mjs";
                               await 0;
```

- ESLint: `packages/p/lib/index.mjs:1:8 n/no-unpublished-import` ("../history.mjs" is not published),
  `packages/p/lib/index.mjs:2:1 n/no-top-level-await`.
- Translation: `packages/p/history.mjs:1 n/no-top-level-await`, `packages/p/lib/index.mjs:2 n/no-top-level-await`; no
  n/no-unpublished-import.

### D10. `private` is read too widely: a string `"true"` and a `"private": true` member of another object skip the package

The plugin skips a package only for `packageJson.private === true`. `pkg_private_skip` (node-resolve.rerail) takes the
string `"true"`, and `npm01_private` takes any line of package.json with `"private": true`, at any depth.

Case c09:

```
a/package.json:     {"name":"a","version":"1.0.0","private":"true","files":["lib"]}
a/lib/index.js:     require("../test/x");
a/test/x.js:        module.exports = 1;
b/package.json:     {"name":"b","version":"1.0.0","files":["lib"],"publishConfig":{
                      "private": true}}
b/lib/index.js:     require("../test/x");
b/test/x.js:        module.exports = 1;
```

- ESLint: `a/lib/index.js:1:9` and `b/lib/index.js:1:9` n/no-unpublished-require.
- Translation: no findings.

### D11. n/no-unpublished-bin is anchored at the first token; ESLint's Program starts at 1:1 in JavaScript files

Since ESLint 9, espree's Program node spans the whole text, comments and the `#!` line included, so the plugin reports a
JavaScript bin file at 1:1. `program_start` (node-package.rerail) takes the first token, which @typescript-eslint/parser
does use: TypeScript files agree, JavaScript files with a shebang or a leading comment are reported on the wrong line.
Most bin files start with `#!/usr/bin/env node`.

Case c13b:

```
package.json:  {"name":"c13b","version":"1.0.0","bin":{"a":"./bin/a.js","c":"./bin/c.ts"},"files":["lib"]}
bin/a.js:      /* cli */
               (blank)
               console.log(1);
bin/c.ts:      #!/usr/bin/env node
               // cli
               console.log(1);
```

- ESLint: `bin/a.js:1:1`, `bin/c.ts:3:1` n/no-unpublished-bin.
- Translation: `bin/a.js:3`, `bin/c.ts:3`.

Case c13 (bin `./bin/a` with `bin/a.js` = `#!/usr/bin/env node`, `// comment`, `console.log(1);`): ESLint `bin/a.js:1:1`,
translation `bin/a.js:3`.

### D12. Minor `files` forms (low impact)

- `files: ["lib/!(a).mjs"]` (fuzz s58): globrex turns `!(a)` into a lookahead that rejects any name starting with `a`, so
  the plugin publishes no file of lib; the translation (`but`) publishes `lib/a.test.mjs`. ESLint: nothing;
  translation: `lib/a.test.mjs:1 n/no-top-level-await`.
- `files: ["lib/sub/../a.mjs"]` (fuzz s65): the plugin normalises the entry (`path.posix.normalize`) to `lib/a.mjs`; the
  translation does not. ESLint: `lib/a.mjs:1:1 n/no-top-level-await`; translation: nothing.

## Checked and agreeing

- n/no-top-level-await's own matching (c10): `for await`, `await using`, `await` in methods, getters, static methods,
  object methods and arrows (not reported), in a labelled block, an `if` body, `export default await`, and in `.mts`.
- `import x = require()` in `.cts` and `require()` in an ES module, also through `createRequire` (c11): neither reports.
- Packages named `chart.js` / `bn.js` imported from TypeScript (c12): both report (the plugin's extension alias also
  rewrites the package name), from JavaScript neither.
- A tsconfig `paths` alias that does not resolve does not fall back to node_modules (c15): both report.
- devDependency-only imports, `import type` of an unpublished file, unresolved `@/x` (unknown kind) in a nested
  package reported as outside it, `~/x` (a package name) not (c14): same findings.
- n/no-unpublished-bin's `isBinFile` forms: `./bin/a` for `bin/a.js`, `bin/b.js`, `./cli` for `cli/index.js` (c13).
- The 44 `.npmignore`/`.gitignore`-only packages of the differential run, and `files` lists with `lib`, `lib/`, `/lib`,
  `./lib`, globs, `**`, `{a,b}`, `@(a|b)`, `src/**/!(*.test).mjs`, `!sub`, `!*.test.mjs`, case, `files` beside
  `.npmignore` or `.gitignore`.

## The author's result file and notes (read after the findings above)

`node-npm-01.result.json` gives all six rules `same` and has no `different` entries, so there is no `right` side to check
against the tool. The corpora the suite compares (fixtures, rerail-typescript) do not contain any of the cases above,
which is why `same` holds there and the defects remain.

NOTES-node-npm-01.md, "What the library does not follow", already discloses D2 (tsconfig `extends`), D4 (subpaths and
`exports`), D7 (`"files": []`) and part of D12 (extglobs, with a different example). D1, D3, D5, D6, D8, D9, D10 and D11
are not in the notes. D10 bears on E3: `package_field` does give a row for the string `"true"`, so node-resolve's
`pkg_private_skip` holds for `"private": "true"`, which the plugin does not skip.

Suite rerun (`ptcheck --box pt-w07 --ref tbelanger/pack-translations-node-npm-01 -- --pack node --rules <the six>`): 6
same, 0 different, 0 failed on fixtures (4528 files) and rerail-typescript (602 files), which matches the result file.
The suite also prints `node does not gate (equivalent: false)` with one problem per rule: "is translated now; remove it
from untranslated".
