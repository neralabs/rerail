# Rerail rule pack: Node.js

No rules yet: the pack ships none and is never recommended. Its 24 ports of eslint-plugin-n 18.4.1 rules, with the
plugin's default options, are candidates ([candidates/README.md](candidates/README.md)). A port starts in
`candidates/rules/` and moves to `rules/` and `pack.json` once it passes the conformance check
([conformance/README.md](../../conformance/README.md#the-gate)); eslint-plugin-n has no upstream manifest
(`upstream/eslint-plugin-n/gates.yaml`) yet, so the check gates none of its ports. Each port is scored against the
plugin's own findings on its rule tests and on their rename variants (`holds-under-rename`), and against the labelled
lines of real-code samples ([docs/upstream.md](../../docs/upstream.md#samples)), through the engine's production
workspace path, with every disagreement declared in `divergences.json` (one today: `n-prefer-promises-fs` does not
report `fs.watch`). The conformance check (`--check`, run in CI) scores the tests and the samples: it gates the ports in
`rules/` and reports the candidates without gating them.

It follows the [pack structure and validation contract](../../docs/pack-format.md) and is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack node"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`); it lists no rules while every port is a candidate |
| `conformance.json` | Each ported rule's upstream rule, how findings compare, and the capabilities it exercises; the pinned plugin |
| `divergences.json` | Declared disagreements with the plugin |
| `upstream/eslint-plugin-n/` | The plugin's copied rule tests (`tests/`), one file per test case (`fixtures/`, their rename variants under `fixtures/renamed/`), its expected findings (`fixtures.expected.json`), the generator (`generate.mjs`, sharing `scripts/pack-data/eslint-expected.mjs`), and the lock naming its real-code samples (`samples.lock`, fetched by `npm run fixtures:pull`) with their sources and licences (`SAMPLES.md`) |
| `candidates/` | Every port (`rules/`, listed in `conformance.json`; `n-no-deprecated-api` and `n-hashbang` in `candidates/conformance.json`) and the Node probes (`probes/`), scored, never shipped; `node_policies.py` writes every document but the hand-written `n-no-process-exit` |
| `candidates/README.md` | What each candidate is waiting on and its rule details; not published |
| `licenses/eslint-plugin-n/` | The plugin's licence and `UPSTREAM.md`, per upstream repository: its source, the patterns of the files taken and how (each file's provenance is `provenance.json` in the tool's test-data archive) |

No rule in `rules/` or `candidates/rules/` pins `languageOptions.sourceType` (the probes do, to state Node facts): each
file keeps its own module system (`.cjs`/`.cts` CommonJS; `.mjs`/`.mts`/`.tsx` an ES module; `.js`/`.jsx`/`.ts` by the
nearest package.json's `type`, Node's syntax detection where that package.json sets no valid `type`, and an ES module
where there is no package.json), as the plugin's `flat/mixed-esm-and-cjs` config picks `flat/recommended-script` or
`flat/recommended-module` per file. Only a CommonJS file has `require`, `module`,
`exports`, `__dirname` and `__filename`; an ES module has none (`flat/recommended-module` turns them off), so the rules
that reach them through scope (exports-style, no-exports-assign, no-path-concat, prefer-global/\*, prefer-promises/\*,
no-deprecated-api) report them in CommonJS files only. Rules upstream reads by name (global-require, no-new-require,
no-mixed-requires, prefer-node-protocol) report in both. The generator lints each fixture the same way:
`globals.node`, without those five names in an ES module. Upstream's RuleTester declares `globals.node` in every case,
so a case it runs as a module that reads one of those names and has no module syntax is written as CommonJS (`.js`
under the fixtures' `"type": "commonjs"` package.json), where Node defines them.

Taking a new snapshot is a maintainer deriving the pack's test data from its pins and publishing it
([docs/upstream.md](../../docs/upstream.md#take-a-new-snapshot)); a pull request carries it only as a publish request
under `upstream/eslint-plugin-n/publish/`. The recipe
(`scripts/pack-data/recipes/node.ts`) runs `upstream/eslint-plugin-n/generate.mjs` against the pinned tools (eslint
10.11.0, eslint-plugin-n 18.4.1, @typescript-eslint/parser 8.70.1, typescript 5.9.3, globals 17.13.0).
Regenerating the rule documents runs offline, against the same tools installed under `.rerail/tools/node`:

Paused: `fixtures:derive` refuses every tool-pack until the generators write per-repo provenance
([docs/upstream.md](../../docs/upstream.md#derive-and-bumps-are-paused)). The derive and stage commands below take a
snapshot once the pause lifts.

```sh
npm run fixtures:derive -- --pack node
npm run fixtures:stage -- --pack node    # copy what changed into upstream/eslint-plugin-n/publish/
python3 scripts/generate/packs/node_policies.py
```

Scoring one rule with every false positive and false negative printed, and the candidates and probes:

```sh
node --import tsx conformance/languages/cli.ts --pack node --rule n-no-process-exit --fixtures
node --import tsx conformance/languages/cli.ts --pack-dir packs/node/candidates --fixtures
node --import tsx conformance/languages/cli.ts --pack-dir packs/node/candidates/probes --fixtures
```
