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

## Git snapshots

The native CLI accepts `--revision <commit-or-tree> --analysis-scope <name>`
for `logic`, `run-rr`, and `run-policies`.
The repository argument selects the object database. A bare repository also works.
The revision can be a full hash, an unambiguous short hash, or a revision expression such as `HEAD~1`.

```sh
nice -n 19 target/debug/rerail-engine run-rr policy.rr /path/to/repo /tmp/report --revision HEAD~1 --analysis-scope review
nice -n 19 target/debug/rerail-engine logic program.rr /path/to/repo /tmp/report --revision 'HEAD^{tree}' --analysis-scope review
nice -n 19 target/debug/rerail-engine delete-scope /path/to/content.sqlite review
```

These commands run from the repository root of this project.
The policy and output paths refer to local files. Each run applies the supplied policy to one selected revision.
If the repository argument names a subdirectory, the run uses that subdirectory from the selected revision.

The engine walks Git trees and retains blob IDs.
Source readers fetch blob bytes directly through `gix` when they need them.
Configuration, declared file inputs, directory listings, and source parsing use the same Git object view.
The view has a stable path identity for cache keys. The engine creates no directory at that path.
Local edits, staged changes, untracked files, and checkout filters do not affect the selected objects.
The scan uses the selected tree's `.rerailignore` and the built-in exclusions.
It does not expand symlinks or submodules.
Snapshot runs reuse content and parsed-source caches. Their whole-run cache keys include the selected object ID.

For a commit, `git_rev("head", sha)` names the selected commit.
`git_rev("tree", sha)` names its root tree.
For a raw tree, only the tree row exists. A raw tree has no commit message or ancestry.
With a commit snapshot, `--base` uses the selected commit to find the merge base.
A raw tree cannot supply a merge base.
`@git_tracked(path)` checks membership in the current index.
`@git_has(sha, path)` accepts both commit and tree IDs.

Native API callers can pass `{"revision": "<commit-or-tree>"}` to `run_rr` or `run_policies`.
The resulting report includes `source.revision`, `source.tree`, and `source.commit`.
See the [Git rule library](../docs/rule-syntax/git.md) for named Git facts and request examples.
