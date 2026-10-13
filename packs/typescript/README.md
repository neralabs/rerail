# Rerail rule pack: TypeScript

9 code rules for TypeScript, ported from typescript-eslint 8.70.1 (on ESLint 10.11.0), each with its default
options. 5 start off (`"on": false`): no approved typescript-eslint configuration turns them on, so they are
experimental. Each is scored against the tool's own findings on the tool's test fixtures and on a pinned corpus
(vscode), through the engine's production workspace path, with every disagreement declared in `divergences.json`: 4
today, all `rerail-unknown` on the candidate `no-unsafe-function-type`, each a file the tree-sitter grammars read with
a syntax error. ESLint's core rules are in
the [JavaScript pack](../javascript/README.md).

It is checked by `npx vitest run test/engine/catalog/packs.test.ts -t "pack typescript"`, against the
[pack structure and validation contract](../../docs/pack-format.md).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title, group and, for a rule that starts off, `on` |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools and the pinned corpus (vscode) |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`) and its recordings (`fixtures.expected.json`, `corpus-vscode.expected.json`) |
| `upstream/generate.mjs` | typescript-eslint's recorder; `upstream/eslint-plugin-vitest/generate.mjs` records @vitest/eslint-plugin |
| `candidates/rules/` | Ports still being evaluated, never shipped: typescript-eslint and @vitest/eslint-plugin 1.6.27 ports that fail a V1 validation gate or rerail's pack contract. Each but `prefer-optional-chain` (it requires type information, which the recordings do not have) and `no-explicit-any` (the fixtures recording does not cover it yet) is listed in `conformance.json`, scored and reported on the fixtures, never gated; the vscode recording covers the shipped rules and the 13 that fail V1 Gate 5 on vscode files the engine cannot fully parse |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, listing every copied file |

Re-recording runs offline against the pinned tool versions (the recorder's header lists them):

```sh
node upstream/generate.mjs --tools <dir> --typescript-eslint <typescript-eslint checkout at v8.70.1> \
  --fixtures --corpus vscode=<vscode checkout>
node upstream/eslint-plugin-vitest/generate.mjs --tools <dir> --vitest <eslint-plugin-vitest checkout at v1.6.27>
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack typescript --rule typescript-eslint-no-namespace --fixtures
```
