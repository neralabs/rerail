# Native engine

The Rust engine, built as a Node addon with napi-rs (`npm run build:native`). It is the only engine source root: the
TypeScript side (`src/engine/`) calls it through `src/engine/native/`.

- `src/code_rule_dsl/` is the code-rule DSL engine: compile, matcher, scope, value, calls, literal, suppress, violations
  and the workspace. Its `capabilities.json` is the grammar of what rules may use; the package ships it, and
  `/rerail-create` points agents at it.
- `src/` (top level) also holds the semantic model loader (`model.rs`), the content-addressed store
  (`content_store/`), the Git workspace index, the process and main-instance helpers, and `build_languages.rs`.
- `build.rs` validates `languages/` and generates the language tables. It also locates `fixtures/` for the engine's
  tests: the one Rust fixtures-root resolver.
- `test-languages/` holds the planned languages only tests see (`exemplar.json`, [docs/language-packs.md](../docs/language-packs.md)):
  `build.rs` generates them as `#[cfg(test)]` items, so no product build knows them.
- `grammars/` holds the vendored tree-sitter grammars, each with its LICENSE, and a PATCHES.md where Rerail carries
  patches in-tree.

Generated files carry `.generated.` in their names, and their generators live in `scripts/generate/` (manifest:
`scripts/generate/README.md`). `value/names/names.bin` is in Git LFS. Run the engine's tests with `npm run test:native`,
and fmt and clippy with `npm run check:native`.
