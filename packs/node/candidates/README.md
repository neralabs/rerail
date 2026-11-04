# Rerail rule pack: Node.js, candidates

The ports in `rules/` here are still being evaluated: scored against the tools' expected findings and reported, never
gated and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `n-no-unsupported-features-es-syntax`

eslint-plugin-n 18.4.1 n/no-unsupported-features/es-syntax (default options: no `ignores`). One clause per eslint-plugin-es-x 7.8.0 rule es-syntax.json lists (199 features; their AST rules in `scripts/generate/packs/node_es_syntax/`), gated on the configured Node range not being a subset of the feature's supported range (`<0` or none: always); a feature with a `strictMode` range (block-scoped functions and variables, classes) runs where the range is not within that range and reports in strict code (a class, a `"use strict"` prologue, a module) only where it is not within the supported range either. Utilities several clauses carry verbatim are shared through one define. Against ESLint the port reports nothing ESLint does not on es-x's own tests, about 1,850 edge cases, 2,829 real JavaScript files and 181 real TypeScript files; each module's `LIMITS` names what it misses: files tree-sitter cannot parse (a tagged template with an invalid escape), escapes after an identifier's start, keys only constant folding resolves, and, in TypeScript, the prototype-method features (`Array.prototype.includes` and the like), whose receivers es-x types with the TypeScript checker.

### `n-no-restricted-import` / `n-no-restricted-require`

eslint-plugin-n 18.4.1 n/no-restricted-import and no-restricted-require, whose default restricts nothing. The targets are the plugin's: an import or export declaration's source, an `import()` string, and the first argument of the global `require` or `require.resolve` (any constant it folds to). The restrictions are the port's `restricted` option, regular expressions each matching a whole module name (the specifier up to its first `!`), where eslint-plugin-n takes globs and a message per entry; without any, nothing is reported.

### `n-file-extension-in-import`

eslint-plugin-n 18.4.1 n/file-extension-in-import (default style `always`, no overrides). Reports a relative or absolute import target that resolves to a file whose expected extension the specifier does not end with: the file's own, a TypeScript file's JavaScript one (`.ts` → `.js`, `.mts` → `.mjs`, …), or, for a folder, its `index` file's (`./dir` expects `./dir/index.js`). A target that resolves to nothing is the other rules' business. No fix is offered. Not ported: the `never` style, per-extension overrides, `tryExtensions` and the `typescriptExtensionMap` option and setting.

### `n-no-extraneous-import`

eslint-plugin-n 18.4.1 n/no-extraneous-import (default options). Reports a package target of an import or export declaration or `import()` that resolves (`resolved: true`) but that neither the importing file's package.json (its own name and its `dependencies`, `devDependencies`, `peerDependencies` and `optionalDependencies`) nor the workspace root that claims it (`workspaces`, or `pnpm-workspace.yaml`) lists (`declared: false`; a type import also counts its `@types` package), and that no tsconfig `paths` key names (`aliased: false`). Without a package.json nothing is checked. Not ported: the `allowModules`, `convertPath`, `resolvePaths`, `resolverConfig` and `tryExtensions` options and their settings.
