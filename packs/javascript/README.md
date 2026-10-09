# Rerail rule pack: JavaScript

Code rules for JavaScript and TypeScript source, ported from ESLint 10.11.0's core rules with their default options.
Each shipped rule is scored against ESLint's own findings on ESLint's test cases, through the engine's production
workspace path, with every disagreement declared in `divergences.json`.

It is checked by `npx vitest run test/packs.test.ts -t "pack javascript"`, against the
[pack structure and validation contract](../../docs/pack-format.md).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group, and when the pack is recommended |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream rule, how findings compare, and the capabilities it exercises; the pinned tool |
| `divergences.json` | Declared disagreements with ESLint |
| `upstream/eslint/` | ESLint's copied test cases (`fixtures/`) and its recording (`fixtures.expected.json`) |
| `upstream/generate.mjs` | The recorder |
| `candidates/rules/` | Ports of ESLint and eslint-plugin-unicorn rules still being evaluated: never shipped |
| `licenses/eslint/` | ESLint's licence and `UPSTREAM.md`, listing every copied file |

The pack is recommended for a repository with JavaScript or TypeScript source files. Files under test, script and
fixture folders, test and spec files, and configuration files (`*.config.*`, `.*rc.*`) do not count.

Re-recording runs offline against the pinned tool versions (the recorder's header lists them):

```sh
node upstream/generate.mjs --tools <dir> --eslint <ESLint checkout at v10.11.0>
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack javascript --rule eslint-use-isnan --fixtures
```
