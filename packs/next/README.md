# Rerail rule pack: Next.js

19 code rules for Next.js projects, ported from `@next/eslint-plugin-next` 16.3.8. Each is scored against the
plugin's own findings on the plugin's rule tests, through the engine's production workspace path; no disagreement is
declared today, so the pack has no `divergences.json`.

The pack follows the [pack structure and validation contract](../../docs/pack-format.md): it is checked by
`npx vitest run test/packs.test.ts -t "pack next"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group; two rules carry examples |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream rule, how findings compare, and the capabilities it exercises; the pinned plugin and the pinned corpora (shadcn-ui, next-examples, commerce) |
| `upstream/eslint-plugin-next/` | The plugin's copied rule tests (`tests/`) and the cases extracted from them (`fixtures/`), its recordings (`fixtures.expected.json`, `corpus-<name>.expected.json`, `fixtures.cases.json`) and the recorder (`generate.mjs`) |
| `candidates/` | Ports that cannot pass their fixtures yet (`conformance.json`, `rules/`), the Next fact model's probes (`probes/`), and the generators that write them and `rules/` (`build-policies.mjs`, `build-probes.mjs`) |
| `licenses/next.js/` | Next.js's licence and `UPSTREAM.md`, listing every copied file |

The rule documents, `conformance.json`'s and `pack.json`'s rule lists and the candidates are generated, offline:

```sh
node packs/next/candidates/build-policies.mjs
node packs/next/candidates/build-probes.mjs
```

Re-recording runs offline against the pinned plugin, with the tools under `.rerail/tools/next` and the corpus checkouts
under `.rerail/corpora/next` (Node 23.6 or later):

```sh
node packs/next/upstream/eslint-plugin-next/generate.mjs
```

Scoring one rule with every false positive and false negative printed, and the candidates:

```sh
node --import tsx conformance/languages/cli.ts --pack next --rule next-no-img-element --fixtures
node --import tsx conformance/languages/cli.ts --pack-dir packs/next/candidates --fixtures
```
