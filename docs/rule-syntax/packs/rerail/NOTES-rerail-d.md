# rerail pack, batch D

Rules: `rerail-grammar-node-kinds`, `rerail-language-branching` (+ `.typescript`), `rerail-language-facts-from-registry`
(+ `.typescript`), `rerail-origin-ranges`. Every translation reports the same findings as its JSON original; nothing is
blocked.

## Library

`lib/rust/rerail-native.rerail` (new): `native_test_item` / `in_native_test` (the JSON's `cfg-test` + `test-item` utils:
an item `#[cfg(test)]` or `#[cfg(all(test, …))]` applies to, read with rust/syntax's `attached`, which skips other
attributes and comments exactly as the JSON's `follows … stopBy: not(attribute or comment)`), `native_label` (a
language label literal) and `family_owned` (the family-module path list that rerail-language-branching and
rerail-language-facts-from-registry share verbatim).

## Per rule

- **rerail-grammar-node-kinds.** The machine translation's 1,102 extra findings came from dropping each clause's
  `file.path` filter. Here the shared-code clause's notRegex is split into its two alternatives: `family_module`
  (a family's own module) and the five dispatching files, and the five after-dispatch clauses are one table,
  `dispatched(file, fn)`, plus the `if self.estree` exception for arguments.rs. `kind-read` is asked only where it is
  used (a comparison's side, a match's subject); that gives the same findings and avoids a regex over every node.
- **rerail-language-branching.** Two policies (Rust, TypeScript). The Rust language and embedding clauses share their
  shapes; `variant(v, c)` / `branch(n, c)` fold them with c = "language" or "embedding". The `not inside
  binary_expression stopBy match_pattern` guard exception uses the variant's nearest match_pattern. The JSON's
  embedding clause copied the `operand` util with its `dialect_by_label("python")` alternative, so
  `grammar.lang == dialect_by_label("tsx")` outside embed/ reports EMBEDDING_BRANCH as well as LANGUAGE_BRANCH. The
  translation keeps that (the test pins it, line 16); the JSON's stated intent (a branch on a `Via`) says the
  EMBEDDING_BRANCH there is spurious, so the JSON is the side that is wrong, but the findings stay identical.
- **rerail-language-facts-from-registry.** Two policies (Rust, TypeScript). The TypeScript policy keeps only the
  excludes that can match a `.ts` file (`**/tests/**`, `**/*.test.ts`, `**/*.generated.ts`); the clause's own
  `src/engine/code-rule/ | src/code-rule*.ts` path regex is a body test.
- **rerail-origin-ranges.** Direct: the AST clause with globs (`host_{range,span}`; the trait `(?:^|::)Origin$` as
  `{Origin,*::Origin}`, equal for a path), the regex clause copied exactly.

Simplification common to all: the JSON's `has` without stopBy (direct children, ast-grep also visits anonymous tokens)
reads named children; every matched node here is named, so no finding changes.

## Validation

`test-rr`: 15 tests, all pass (grammar-node-kinds 4, language-branching 3 + 2, facts 3 + 2, origin-ranges 2). Every
test's fixtures were also run as a corpus through the JSON originals: same findings for all 15.

| rule | caught | passes | repo corpus (7efcac9c67) |
|---|---|---|---|
| rerail-grammar-node-kinds | same (1) | same (0) | same (89) |
| rerail-language-branching | same (1) | same (0) | same (109) |
| rerail-language-facts-from-registry | same (1) | same (0) | same (17) |
| rerail-origin-ranges | same (1) | same (0) | same (2) |

`site/check_rr.py`: no problem in these files (its 14 problems are pre-existing, elsewhere).
