# Rerail rule pack: React

Ports of eslint-plugin-react 7.37.5 and eslint-plugin-react-hooks 7.1.1 rules. A port starts in `candidates/rules/`
and moves to `rules/` and `pack.json` once it passes the V1 rule validation gates
([nera-rule-validation](https://github.com/neralabs/nera-rule-validation)). While `pack.json` lists no rules, the pack
ships none and is never recommended. The ports written from `candidates/specs.mjs` are scored against the plugins' own
findings on the plugins' test cases, through the engine's production workspace path;
`candidates/rules/react-no-array-index-key.json` is a hand-written port with no recording.

The pack follows the [pack structure and validation contract](../../docs/pack-format.md): it is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack react"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): its match definition, and no rules until a port passes |
| `upstream/<tool>/` | The plugin's copied tests (`tests/`), the cases extracted from them (`fixtures/`), its recordings (`fixtures.expected.json`, `corpus-excalidraw.expected.json`) and the recorder (`generate.mjs`, sharing `upstream/common.mjs` and `conformance/recorders/eslint-record.mjs`) |
| `licenses/<project>/` | The upstream licence and `UPSTREAM.md`, listing every copied file and extracted case |
| `candidates/` | Ports still being evaluated, never shipped: the code-rule documents (`rules/`, written by `build.mjs` from `specs.mjs`), and `conformance.json`, each recorded port's upstream tool and rule, how findings compare and the capabilities it exercises, with the pinned tools and corpus (excalidraw), scored against `upstream/`; `probes/` holds the hand-labelled React fact probes |

Re-recording runs offline against the pinned tools (eslint 9.39.5, @typescript-eslint/parser 8.70.1, typescript
5.9.3) and the plugins' upstream checkouts, installed under `.rerail/tools/react`, with the corpus checkout under
`.rerail/corpora/react`:

```sh
node upstream/eslint-plugin-react/generate.mjs [--tools <dir>] [--corpora <dir>] [--no-corpora]
node upstream/eslint-plugin-react-hooks/generate.mjs [--tools <dir>] [--corpora <dir>] [--no-corpora]
```

Rewriting the rule documents after editing `candidates/specs.mjs`, then scoring one rule with every false positive
and false negative printed:

```sh
node candidates/build.mjs
node --import tsx conformance/languages/cli.ts --pack-dir packs/react/candidates --rule react-jsx-key --fixtures
```
