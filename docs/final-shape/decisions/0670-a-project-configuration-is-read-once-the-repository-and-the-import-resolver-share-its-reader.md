# 0670. A project configuration is read one way: the repository and the import resolver share its reader

Status: decided (2026-11-23). It follows 0652, whose packs test found the node and react packs reading
`tsconfig_for`, which core declares and nothing made (O80).

## Intro

A TypeScript or JavaScript project's configuration (`tsconfig.json`, `jsconfig.json`) says where a file's imports
resolve: its `baseUrl` and `paths`, merged over the configurations it `extends`. Two parts of the engine read it:

- the import resolver (`lang_ecma`'s `import_target::tsconfig`), get-tsconfig's reading over a workspace's held
  files and the disk below them;
- the repository reader (`facts::repository`), which makes the `tsconfig`, `tsconfig_path` and `tsconfig_for` rows
  that rules read.

0652's fix gave the repository reader a reading of its own: a JSONC stripper, an `extends` chain of relative paths
only, and path joins. That was a second copy of the import resolver's, and a weaker one: a base in `node_modules`
(`"extends": "@tsconfig/node20"`, the common case) was not followed, and a byte-order mark failed the parse.

## Decision

1. **One reader, in `rerail-lang`** (`code_rule_dsl::tsconfig`), below both crates: `strip_jsonc` (a byte-order
   mark, comments, then trailing commas, as TypeScript reads one), `extends_of`, and `extends_path`, get-tsconfig's
   `resolveExtendsPath` (a relative or absolute path, `.json` appended where it names no file, or a package's
   configuration in a `node_modules` folder at or above, by its `tsconfig` field, a subpath, or its
   `tsconfig.json`).
2. **Where the files come from is the caller's** (`ConfigFiles`): the import resolver's `View` (held files over
   disk), the repository's `Disk` (the files below its root). A path resolved outside the root is not read, and a
   package linked into `node_modules` is read where it resolves, spelled from the root.
3. **Node's POSIX path functions move with it** (`code_rule_dsl::node_path`, from `import_target::path`, which
   re-exports them): one `join`, `resolve` and `dirname` for both.
4. Each side keeps its own merge, as each reads a different value: the resolver its ordered entries
   (`compilerOptions` as get-tsconfig spreads them), the repository its rows, with each `baseUrl` from the
   configuration that sets it.

## Tests

- `a_project_configuration_is_read_with_the_chain_it_extends` (`facts::repository`): JSONC with a byte-order mark
  and trailing commas, a base without `.json`, a scoped package's configuration by its `tsconfig` field, later bases
  over earlier ones and the file's own options over both, `tsconfig_for` of a document below it and none for one
  outside it.
- The import resolver's tests (`import_target::tests`) and the path functions' (`node_path::tests`) as they were.
