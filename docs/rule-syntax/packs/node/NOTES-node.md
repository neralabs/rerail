# packs/node: eslint-plugin-n rules, hand-written

These are 36 rules from eslint-plugin-n 18.4.1, checked against ESLint 9.39.

- **Scores** come from `tools/rrlab/rrlab score node <id> --all` and are given as TP/FP/FN on (path, line, code).
- **"not recorded"** means the recording has no fixtures for the rule. Those rules were checked case by case against
  ESLint with the plugin itself.

The shared vocabulary lives in four libraries:

- `lib/tsx/node.rr`: CommonJS view, calls, ESTree ancestors, `node_runs_on` and the built-in module table.
- `lib/tsx/node-package.rr`: package.json, bin, shebang and the Program start.
- `lib/tsx/node-resolve.rr`: module specifiers and resolution.
- `lib/tsx/node-trace.rr`: ReferenceTracker uses of a built-in module, built on values.rr.

Two rules read generated tables, `n-no-deprecated-api.facts.rr` and `n-no-unsupported-features-es-builtins.facts.rr`.
Both come from `gen-node-tables.py --plugin DIR --globals globals.json`, which reads the plugin's own trace maps.

| rule | lines | TP/FP/FN | deviations |
|---|---|---|---|
| n-callback-return | 49 | 24/0/0 | |
| n-exports-style | 42 | 29/0/0 | |
| n-global-require | 31 | 19/0/0 | D1 |
| n-handle-callback-err | 28 | 19/0/0 | |
| n-hashbang | 48 | 37/0/1 | D3, D4 |
| n-no-callback-literal | 31 | 10/0/0 | |
| n-no-deprecated-api | 65 | 121/0/1 | D5 |
| n-no-exports-assign | 29 | 6/0/0 | |
| n-no-extraneous-require | 22 | not recorded | D6, D4 |
| n-no-missing-import | 20 | not recorded (0/11/0 with --all) | D6 |
| n-no-missing-require | 20 | not recorded (0/51/0 with --all) | D6 |
| n-no-mixed-requires | 76 | 5/0/0 | |
| n-no-new-require | 17 | 2/0/0 | |
| n-no-path-concat | 73 | 20/0/0 | D7 |
| n-no-process-env | 30 | 16/0/0 | |
| n-no-process-exit | 19 | 8/0/0 | |
| n-no-sync | 30 | 23/0/0 | D8 |
| n-no-top-level-await | 30 | not recorded | D9, D4 |
| n-no-unpublished-bin | 17 | not recorded | D9, D4 |
| n-no-unpublished-import | 17 | not recorded | D6, D9, D4 |
| n-no-unpublished-require | 18 | not recorded | D6, D9, D4 |
| n-no-unsupported-features-es-builtins | 22 | not recorded | D10 |
| n-prefer-global-buffer | 18 | 45/0/0 | D12 |
| n-prefer-global-console | 18 | 12/0/0 | |
| n-prefer-global-crypto | 18 | not recorded | |
| n-prefer-global-process | 18 | 10/0/0 | |
| n-prefer-global-text-decoder | 18 | 12/0/0 | |
| n-prefer-global-text-encoder | 18 | 12/0/0 | |
| n-prefer-global-timers | 21 | not recorded | |
| n-prefer-global-url | 18 | 12/0/0 | |
| n-prefer-global-url-search-params | 18 | 12/0/0 | |
| n-prefer-import-assert-strict | 62 | not recorded | |
| n-prefer-node-protocol | 58 | 279/0/0 | |
| n-prefer-process-get-builtin-module | 60 | not recorded (0/229/0 with --all) | D11 |
| n-prefer-promises-dns | 28 | 27/0/0 | |
| n-prefer-promises-fs | 28 | 40/0/0 | |

In all, the rules are 1135 lines, the four libraries 501 lines and the two generated tables 919 lines.

The "with --all" FPs for the rules that are not recorded are not real errors. They come from fixtures that ESLint
never ran those rules on:

- For no-missing-\*, every specifier fails to resolve, because resolution has no rows yet (D6).
- For get-builtin-module, they are the other rules' `require("fs")` lines.

## Deviations

- **D1** global-require: TypeScript-only forms (`import x = require()`, satisfies/as wrappers) are not in the plugin's
  list of accepted parents and are not checked.
- **D3** hashbang: invalid-026 has two findings at the same key. The recording counts both and rrlab counts one, so the
  1 FN is that artefact. A `#!` line with no line break after it is no shebang, as in the plugin.
  additionalExecutables are gitignore patterns (node-package.rr's `pkg_listed`: anchoring, directories, `!`), and a
  file they match is checked even under ignoreUnpublished.
- **D4** `convertPath`, `settings.n` and `pnpm-workspace.yaml` are not read. Only the `version` option and
  package.json are read.
- **D5** deprecated-api: invalid-101 is `GLOBAL; /*globals GLOBAL*/`. rrlab gives `directive_global` no rows (R1), so
  the rule's clause for comment-declared globals never fires. ESLint gives several findings at one key for
  `export * from "fs"`, and the rule gives one.
- **D6** resolution (`spec_resolved`) and npm's published view (`npm_ignored`) are PROPOSED native inputs (C1, C2) with
  no rows today. Every specifier then fails to resolve, and every file counts as published. The rule logic around them
  matches the plugin case by case: which specifiers, style, allowModules, the TS alias, devDependency-only, private.
- **D7** no-path-concat: `path.sep` is taken as "/" (POSIX), as on the recording machine.
- **D8** no-sync: the typed `ignores` entries (`{from: "file"|"lib"|"package"}`) need types and are not ported. String
  entries are.
- **D9** no-unpublished-\* and no-top-level-await: `ignorePrivate` needs `package_field(dir, "private", "true")`. rrlab
  emits booleans as scalars, but core does not document them (C3).
- **D10** es-builtins: names are followed as values.rr follows them. The ESLint globals come from npm `globals`
  (`builtin` plus `node`), which may differ from ESLint's own ES globals list for a very new ecmaVersion.
- **D11** get-builtin-module: a module name is constant as getStaticValue folds it without a scope: a literal, a
  template of constants, a `+` of them, `true ? a : b`. Other folds (`!0 ? …`, `String.raw`) are not modelled.
- **D12** trace rules (node-trace.rr): a file that writes the global `require` (`require = f`) still has its
  `require("m")` traced (C7); a written global itself (`Buffer = 1`) is not.

## Library and core change requests

**C1. core: module resolution.** Add this native input:

```
.decl spec_resolved(n: node, file: symbol)   // string n (a require/import specifier) resolves to file, as
.input spec_resolved                         // enhanced-resolve does with eslint-plugin-n's options; no row = fails
```

Today it is declared in `lib/tsx/node-resolve.rr`, which also gives the options it needs: the conditions, the
tryExtensions, the tsconfig paths and the .js->.ts map.

**C2. core: npm's publish view.** Add this native input:

```
.decl npm_ignored(path: symbol)   // npm leaves path out of the package: package.json `files`, else .npmignore,
.input npm_ignored                // else .gitignore; README/LICENSE/CHANGELOG/package.json/main always in
```

Today it is declared in `lib/tsx/node-package.rr`.

**C3. core: package_field booleans.** Document `package_field` so that booleans and numbers give their JSON text, as in
`package_field(dir, "private", "true")`.

**C4. names.rr: one declaration per reference.** Done in 5e3da9d0b; handle-callback-err no longer works around it.

**C5. values.rr: comment-declared globals.** Line 137 should not need `node_global(g)`. The rule's own table already
bounds `api_known`. The new line:

```
env_global(r, g) :- ref(r, g, "value"), !ref_site(r, _), docof(r, d), directive_global(d, g).
```

This is because ESLint declares `/* global GLOBAL */` names in the global scope, so ReferenceTracker follows them.

**C6. check_rr: false "unbound x".** It reports `unbound x` on a disjunction whose second branch binds the variable by
equality: `p(x) :- q(x) ; x = "c".` n-prefer-promises-dns was split into two clauses to get past it.

**C7. values.rr: the forms ReferenceTracker follows.** node-trace.rr adds these for the node rules; they belong in
values.rr, after `loader` and the alias clauses of `val`:

```
loader(f) :- val(f, "require", "api").   // const r = require; r("fs"), global.require("fs")
val(n, p, k) :- kind(n, "binary_expression"), (op(n, "||") ; op(n, "&&") ; op(n, "??")),
    (field(n, "left", v) ; field(n, "right", v)), val(v, p, k).
val(n, p, k) :- kind(n, "ternary_expression"), (field(n, "consequence", v) ; field(n, "alternative", v)),
    val(v, p, k).
val(n, p, k) :- kind(n, "sequence_expression"), child(n, v, i), !child(n, _, i + 1), val(v, p, k).
val(n, p, k) :- (kind(n, "as_expression") ; kind(n, "satisfies_expression") ; kind(n, "non_null_expression")),
    child(n, v, 0), val(v, p, k).
```

String import names (`import { "Buffer" as B } from "buffer"`): `import_binding` and the specifier clause of
`api_use` read `text(n, x)`, quotes included; they should take `value(n, x)` when n is a string. And a global the
file writes is not traced (isModifiedGlobal). Add `docof(r, d), !env_written(d, g)` to each `env_global` clause, with

```
.decl env_written(d: number, g: symbol)  // `g = …`, `g += …`, `g++` with no declaration of g
env_written(d, g) :- (kind(a, "assignment_expression") ; kind(a, "augmented_assignment_expression")),
    field(a, "left", l), kind(l, "identifier"), text(l, g), !ref_site(l, _), docof(a, d).
env_written(d, g) :- kind(a, "update_expression"), child(a, l, _), kind(l, "identifier"), text(l, g),
    !ref_site(l, _), docof(a, d).
```

node-trace.rr filters `global_use` this way today; the module side needs it in values.rr.

**rrlab notes:**

- **R1** `directive_global` has no rows, and no "not modelled" warning says so.
- **R2** `places.tsv` is appended across runs, so stale ids appear until the file is removed.
- **R3** `package_field` emits every scalar, and keys are not `~1`-escaped as core documents them.
- **R4** findings that share a key are counted separately on the recording side.
- **R5** `src_line` keeps the BOM, and the `hash_bang_line` text keeps `\r`. std::regex `\uFEFF` did not match UTF-8
  (fixed: rrlab uses Rust regex since 08d977aa5; n-hashbang's `src_bom` reads the column, not a regex).

## Duplicates to merge

- `node_call` in `node.rr` and `called` in `eslint-b-scope.rr`: both are a call's callee with parentheses skipped.
- `cjs_ref` in `node.rr` and `env_global` in `values.rr` cover the CommonJS globals. `cjs_ref` also allows
  `/* global */`.
- `node_runs_on`/`node_version_opt` in `node.rr` duplicate `runs_on`/`version_opt` in
  n-no-unsupported-features-node-builtins.rr. The exemplar could use the library.
- `node_builtin` in `node.rr` (Node's builtinModules) and the exemplar facts' `node_module` rows: one table could give
  both.
- `const_string` in n-prefer-process-get-builtin-module is values.rr's `const_str` plus getStaticValue's folds (`+`,
  templates, `true ? a : b`). values.rr could take them.
- `pkg_listed`'s gitignore matcher in `node-package.rr` is the logic `npm_ignored` (C2) needs for .npmignore; a
  native matcher would serve both.
- `trace_written` in `node-trace.rr` and `global_written` in `eslint-b-scope.rr`.
- The `literal()` parser in `gen-node-tables.py` and the JSON inversion in `gen-node-builtins.py` both read
  plugin trace maps. The exemplar's generator could read the plugin's literal directly, as this one does.
