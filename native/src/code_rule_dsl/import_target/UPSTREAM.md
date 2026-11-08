# enhanced-resolve 5.26.0, get-tsconfig 4.14.3, eslint-plugin-n 18.4.1 `ImportTarget`

Upstream:

- [webpack/enhanced-resolve](https://github.com/webpack/enhanced-resolve/tree/c65e40ac1b77a58d1bcbc883953974236991a833) at tag `v5.26.0` (commit `c65e40ac1b77a58d1bcbc883953974236991a833`). License: MIT (Copyright JS Foundation and other contributors).
- [privatenumber/get-tsconfig](https://github.com/privatenumber/get-tsconfig/tree/b4e7751e408eeaf014974a7cb1ec2634914a4d6c) at tag `v4.14.3` (commit `b4e7751e408eeaf014974a7cb1ec2634914a4d6c`). License: MIT (Copyright (c) Hiroki Osame <hiroki.osame@gmail.com>).
- [eslint-community/eslint-plugin-n](https://github.com/eslint-community/eslint-plugin-n/tree/f1d47aff402b9e009578f4b3155b84f7d8beef9b) at tag `v18.4.1` (commit `f1d47aff402b9e009578f4b3155b84f7d8beef9b`). License: MIT (Copyright (c) 2015 Toru Nagashima).

The licence text is [`LICENSES/MIT.txt`](../../../../LICENSES/MIT.txt); the copyright notices and full texts are in [`THIRD_PARTY_NOTICES`](../../../../THIRD_PARTY_NOTICES).

The files below are under MIT, as `REUSE.toml` annotates them. The rest of this directory is Rerail's (Apache-2.0): `fs.rs` (the files a resolution reads), `json.rs` (JSON as `JSON.parse` orders keys), `path.rs` (Node's POSIX `path` functions, written to their documented behaviour) and the tests (`tests.rs`).

Paths are relative to this directory, `native/src/code_rule_dsl/import_target/`.

| File | Upstream path | Copy |
|---|---|---|
| `resolver.rs` | enhanced-resolve `lib/ResolverFactory.js`, `lib/Resolver.js`, `lib/AliasUtils.js`, `lib/DescriptionFileUtils.js`, `lib/getPaths.js`, `lib/util/identifier.js`, `lib/util/path.js`, and the plugins `createResolver` taps: `AliasPlugin`, `AppendPlugin`, `ConditionalPlugin`, `DescriptionFilePlugin`, `DirectoryExistsPlugin`, `ExportsFieldPlugin`, `ExtensionAliasPlugin`, `FileExistsPlugin`, `ImportsFieldPlugin`, `JoinRequestPartPlugin`, `JoinRequestPlugin`, `MainFieldPlugin`, `ModulesInHierarchicalDirectoriesPlugin`, `NextPlugin`, `ParsePlugin`, `ResultPlugin`, `SelfReferencePlugin`, `SymlinkPlugin`, `TryNextPlugin`, `UseFilePlugin` (`lib/<name>.js`) | derived: translated to Rust: the synchronous resolver's hooks and plugins in their order, for the options eslint-plugin-n's `ImportTarget` sets and enhanced-resolve's defaults for the rest, over the files a workspace or snapshot holds and the files on disk below its root; past 20,000 `doResolve` steps a resolution fails |
| `entrypoints.rs` | enhanced-resolve `lib/util/entrypoints.js` | derived: translated to Rust: the `exports` and `imports` field matching under a set of condition names |
| `tsconfig.rs` | get-tsconfig `src/get-tsconfig.ts`, `src/parse-tsconfig/index.ts`, `src/parse-tsconfig/resolve-extends-path.ts` | derived: translated to Rust: the nearest `tsconfig.json` at or above a file, never above the root, and the `compilerOptions` of the configurations it `extends` (at most 8 deep) beneath its own; the rest of the configuration is not read |
| `mod.rs` | eslint-plugin-n `lib/util/import-target.js`, `visit-import.js`, `visit-require.js`, `get-try-extensions.js`, `get-typescript-extension-map.js`, `get-tsconfig.js`, `get-resolve-paths.js`, `get-package-json.js`, `get-pnpm-workspace-yaml.js`, `strip-import-path-params.js`, `check-publish.js`, `check-extraneous.js`; `lib/rules/file-extension-in-import.js` | derived: translated to Rust at the plugin's default options: a target's module type, name, style and file path, the extension `file-extension-in-import` expects (`always`), and `check-publish`'s and `check-extraneous`'s reads of the importing package |
| `term.rs` | eslint-plugin-n `lib/util/import-target.js` (`getModuleStyle`), `lib/util/visit-import.js`, `lib/util/visit-require.js` | derived: translated to Rust: the `importTarget` matcher term, reading a specifier node's style from the statement around it and its name as `String()` of its constant value |
| `builtins.rs` | eslint-plugin-n `lib/unsupported-features/node-builtins.js` | excerpt: the keys of `NodeBuiltinModules`, in the table's order, beside Node 26.5.0's `module.isBuiltin` names |
