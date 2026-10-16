# Rerail rule pack: Node.js

24 ports of eslint-plugin-n 18.4.1 rules with its default options. A port starts in `candidates/rules/` and moves to
`rules/` and `pack.json` once it passes the port readiness gates ([docs/port-readiness.md](../../docs/port-readiness.md)); eslint-plugin-n
has no gates recording yet (NER-1898), so every port is a candidate today, `pack.json` lists no rules, and the pack ships
none and is never recommended. Each port is scored against the plugin's own findings on its rule tests and on two pinned
corpora (fastify, prettier), through the engine's production workspace path, with every disagreement declared in
`divergences.json` (one today: `n-prefer-promises-fs` does not report `fs.watch`). The conformance check (`--check`, run
in CI) scores the tests: it gates the ports in `rules/` and reports the candidates without gating them.

It follows the [pack structure and validation contract](../../docs/pack-format.md) and is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack node"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`); it lists no rules while every port is a candidate |
| `conformance.json` | Each ported rule's upstream rule, how findings compare, and the capabilities it exercises; the pinned plugin and the pinned corpora (fastify, prettier) |
| `divergences.json` | Declared disagreements with the plugin |
| `upstream/eslint-plugin-n/` | The plugin's copied rule tests (`tests/`), one file per test case (`fixtures/`), its recordings (`fixtures.expected.json`, `corpus-<name>.expected.json`) and the recorder (`generate.mjs`, sharing `conformance/recorders/eslint-record.mjs`) |
| `candidates/` | Every port (`rules/`, listed in `conformance.json`; `n-no-deprecated-api` and `n-hashbang` in `candidates/conformance.json`) and the Node probes (`probes/`), scored, never shipped; `node_policies.py` writes every document but the hand-written `n-no-process-exit` |
| `licenses/eslint-plugin-n/` | The plugin's licence and `UPSTREAM.md`, listing every copied file |

No rule in `rules/` or `candidates/rules/` pins `languageOptions.sourceType` (the probes do, to state Node facts): each
file keeps its own module system (`.cjs`/`.cts` CommonJS; `.mjs`/`.mts`/`.tsx` an ES module; `.js`/`.jsx`/`.ts` by the
nearest package.json's `type`, Node's syntax detection where that package.json sets no valid `type`, and an ES module
where there is no package.json), as the plugin's `flat/mixed-esm-and-cjs` config picks `flat/recommended-script` or
`flat/recommended-module` per file. Only a CommonJS file has `require`, `module`,
`exports`, `__dirname` and `__filename`; an ES module has none (`flat/recommended-module` turns them off), so the rules
that reach them through scope (exports-style, no-exports-assign, no-path-concat, prefer-global/\*, prefer-promises/\*,
no-deprecated-api) report them in CommonJS files only. Rules upstream reads by name (global-require, no-new-require,
no-mixed-requires, prefer-node-protocol) report in both. The recorder lints each fixture and corpus file the same way:
`globals.node`, without those five names in an ES module. Upstream's RuleTester declares `globals.node` in every case,
so a case it runs as a module that reads one of those names and has no module syntax is written as CommonJS (`.js`
under the fixtures' `"type": "commonjs"` package.json), where Node defines them.

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
