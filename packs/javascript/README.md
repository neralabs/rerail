# Rerail rule pack: JavaScript

71 code rules for JavaScript and TypeScript source, ported from ESLint 10.11.0's core rules with their default options.
24 start on; 47 start off (`"on": false`) because `eslint:recommended` does not turn them on, so they are experimental.
Each shipped rule is scored against ESLint's own findings on ESLint's test cases, through the engine's production
workspace path, with every disagreement declared in `divergences.json` (none today).

It is checked by `npx vitest run test/engine/catalog/packs.test.ts -t "pack javascript"`, against the
[pack structure and validation contract](../../docs/pack-format.md).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group, and when the pack is recommended |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's (and candidate's) upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools |
| `divergences.json` | Declared disagreements with ESLint |
| `upstream/eslint/` | ESLint's copied test cases (`fixtures/`) and its recording (`fixtures.expected.json`) |
| `upstream/generate.mjs` | ESLint's recorder |
| `upstream/eslint-plugin-unicorn/` | eslint-plugin-unicorn's copied test cases, its recording and its recorder (`generate.mjs`) |
| `upstream/<tool>/gates.yaml`, `recording.lock`, `goldens.tsv` | The port readiness gates' manifest for the tool, the lock naming its recordings (`recording.json`, `corpus.json`, held in a GitHub release) and its goldens ([docs/port-readiness.md](../../docs/port-readiness.md)) |
| `candidates/rules/` | Ports of ESLint and eslint-plugin-unicorn rules still being evaluated: listed in `conformance.json`, scored and reported, never gated, never shipped |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, listing every copied file |

The pack is recommended for a repository with JavaScript or TypeScript source files. Files under test, script and
fixture folders, test and spec files, and configuration files (`*.config.*`, `.*rc.*`) do not count.

This pack and the [TypeScript pack](../typescript/README.md) split ECMA files by type. The 21 core rules that
typescript-eslint 8.70.1 replaces with an extension rule, or turns off for TypeScript in its `eslint-recommended`
configuration (`no-redeclare`, `no-undef`, `no-unused-vars`, `no-dupe-keys` and others), run on `.js`, `.jsx`, `.mjs`
and `.cjs` files only, and are scored on ESLint's JavaScript test cases only (`fixtures` in `conformance.json`). On
TypeScript they report overloads, declaration merges and type names, and with both packs added a typescript-eslint
port would report the same violation again. `no-loss-of-precision` and `prefer-spread`, which typescript-eslint leaves
to the core rule on TypeScript, run on every ECMA file.

Re-recording runs offline against the pinned tool versions (the recorder's header lists them):

```sh
node upstream/generate.mjs --tools <dir> --eslint <ESLint checkout at v10.11.0>
node upstream/eslint-plugin-unicorn/generate.mjs --tools <dir> --unicorn <eslint-plugin-unicorn checkout at v76.0.0>
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack javascript --rule eslint-use-isnan --fixtures
```
