# Rerail rule pack: TypeScript

36 code rules for TypeScript and JavaScript, ported from ESLint 10.11.0 (14 rules) and typescript-eslint 8.70.1
(22 rules), each with its default options. 11 typescript-eslint rules start off (`"on": false`): no approved
typescript-eslint configuration turns them on, so they are experimental. Each is scored against the tool's own findings on the tool's test fixtures
and on a pinned corpus (vscode), through the engine's production workspace path, with every disagreement declared in
`divergences.json`: 14 today, all `rerail-unknown`, each a file the tree-sitter grammars read with a syntax error.

It is checked by `npx vitest run test/engine/catalog/packs.test.ts -t "pack typescript"`, against the
[pack structure and validation contract](../../docs/pack-format.md).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title, group and, for a rule that starts off, `on`; two rules carry examples |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools and the pinned corpus (vscode) |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`) and its recordings (`fixtures.expected.json`, `corpus-vscode.expected.json`) |
| `upstream/generate.mjs` | The recorder for both tools |
| `candidates/rules/` | Ports still being evaluated, never shipped: typescript-eslint and @vitest/eslint-plugin 1.6.27 ports that fail a V1 validation gate or rerail's pack contract |
| `candidates/typescript-eslint-no-explicit-any.json` | A candidate still in the older `tempo.policy/v1` format, which is not a rule document, so it stays out of `rules/` |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, listing every copied file |

Re-recording runs offline against the pinned tool versions (the recorder's header lists them):

```sh
node upstream/generate.mjs --tools <dir> --eslint <ESLint checkout at v10.11.0> \
  --typescript-eslint <typescript-eslint checkout at v8.70.1> --fixtures --corpus vscode=<vscode checkout>
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack typescript --rule eslint-eqeqeq --fixtures
```
