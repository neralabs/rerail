# Rerail rule pack: React

18 code rules for React, ported from eslint-plugin-react 7.37.5 (17 rules) and eslint-plugin-react-hooks 7.1.1
(`rules-of-hooks`). Each is scored against the plugin's own findings on the plugin's test cases, through the engine's
production workspace path, with every disagreement declared in `divergences.json` (none today).

The pack follows the [pack structure and validation contract](../../docs/pack-format.md): it is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack react"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group; two rules carry examples |
| `rules/` | The code-rule documents (`rerail.policy/v1`), written by `candidates/build.mjs` from `candidates/specs.mjs` |
| `conformance.json` | Each ported rule's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools and the pinned corpus (excalidraw) |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The plugin's copied tests (`tests/`), the cases extracted from them (`fixtures/`), its recordings (`fixtures.expected.json`, `corpus-excalidraw.expected.json`) and the recorder (`generate.mjs`, sharing `upstream/common.mjs` and `conformance/recorders/eslint-record.mjs`) |
| `licenses/<project>/` | The upstream licence and `UPSTREAM.md`, listing every copied file and extracted case |
| `candidates/` | Ports still being evaluated (`react-in-jsx-scope`, `exhaustive-deps`), scored against `upstream/` but never shipped; `probes/` holds the hand-labelled React fact probes |

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
node --import tsx conformance/languages/cli.ts --pack react --rule react-jsx-key --fixtures
```
