# Rerail rule pack: Node.js

22 code rules for Node.js, ported from eslint-plugin-n 18.4.1 with its default options. Each is scored against the
plugin's own findings on its rule tests and on two pinned corpora (fastify, prettier), through the engine's production
workspace path, with every disagreement declared in `divergences.json` (none today).

It follows the [pack structure and validation contract](../../docs/pack-format.md) and is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack node"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group; two rules carry examples |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream rule, how findings compare, and the capabilities it exercises; the pinned plugin and the pinned corpora (fastify, prettier) |
| `divergences.json` | Declared disagreements with the plugin |
| `upstream/eslint-plugin-n/` | The plugin's copied rule tests (`tests/`), one file per test case (`fixtures/`), its recordings (`fixtures.expected.json`, `corpus-<name>.expected.json`) and the recorder (`generate.mjs`, sharing `conformance/recorders/eslint-record.mjs`) |
| `candidates/` | Ports still being evaluated (`n-no-deprecated-api`, `n-hashbang`) and the Node probes (`probes/`), each a conformance pack scored, never shipped; `generate.py` writes their documents and every generated rule in `rules/` |
| `licenses/eslint-plugin-n/` | The plugin's licence and `UPSTREAM.md`, listing every copied file |

Re-recording and regenerating run offline against the pinned tools under `.rerail/tools/node` (eslint 10.11.0,
eslint-plugin-n 18.4.1, @typescript-eslint/parser 8.70.1, typescript 5.9.3, globals 17.13.0) and the corpus
checkouts under `.rerail/corpora/node`:

```sh
node packs/node/upstream/eslint-plugin-n/generate.mjs [--fixtures-only]
python3 scripts/generate/packs/node_policies.py
```

Scoring one rule with every false positive and false negative printed, and the candidates and probes:

```sh
node --import tsx conformance/languages/cli.ts --pack node --rule n-no-process-exit --fixtures
node --import tsx conformance/languages/cli.ts --pack-dir packs/node/candidates --fixtures
node --import tsx conformance/languages/cli.ts --pack-dir packs/node/candidates/probes --fixtures
```
