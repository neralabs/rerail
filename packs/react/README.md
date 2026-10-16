# Rerail rule pack: React

Ports of eslint-plugin-react 7.37.5 and eslint-plugin-react-hooks 7.1.1 rules. A port starts in `candidates/rules/` and
moves to `rules/` and `pack.json` once it passes the [port readiness gates](../../docs/port-readiness.md), which CI runs on every
port. While `pack.json` lists no rules, the pack ships none and is never recommended. Every port is scored against the
plugins' own findings on the plugins' test cases and the pinned corpus, through the engine's production workspace path.
The conformance check (`--check`, run in CI) scores the test cases only: it gates the ports in `rules/` and reports the
candidates without gating them. The corpus is scored on demand, with `--pack react --corpus excalidraw`.

Every rule runs on JavaScript and TypeScript files whose JSX runtime is React or not known; a file whose JSX belongs to
another library is skipped. The runtime is the file's `@jsxImportSource` pragma when it names React, Emotion
(`@emotion/react`) or Theme UI (React) or Preact, Solid, Vue, Stencil, Hono, Qwik, Inferno, Mithril or Hyperapp (another
library), or its `@jsx` pragma when that names `React`; else an `h` or `createElement` imported from Stencil, Preact,
Vue, Solid, Hyperapp or Mithril (another library); else its nearest `package.json`: React when it depends on react,
react-dom, react-native or next and on none of vue, @stencil/core, preact, solid-js, @builder.io/qwik, inferno or
mithril, another library in the reverse case, unknown otherwise. `react-in-jsx-scope` also skips files whose `@jsx`
pragma names another factory (Emotion's `jsx`, for one): the plugin then asks for that factory in scope, which the
port does not check.

The pack follows the [pack structure and validation contract](../../docs/pack-format.md): it is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack react"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): its match definition, and no rules until a port passes |
| `conformance.json` | Every port (shipped under `rules/`, or a candidate under `candidates/rules/`): its upstream tool and rule, how findings compare and the capabilities it exercises; the pinned tools and the pinned corpus (excalidraw). Written by `candidates/build.mjs` |
| `upstream/<tool>/` | The plugin's copied tests (`tests/`), the cases extracted from them (`fixtures/`), its recordings (`fixtures.expected.json`, `corpus-excalidraw.expected.json`) and the recorder (`generate.mjs`, sharing `upstream/common.mjs` and `conformance/recorders/eslint-record.mjs`); `UPSTREAM.md` lists the fixtures Rerail wrote beside them (`common.mjs` `AUTHORED`): files of other JSX libraries, which the plugins report and the ports skip, and React files whose JSX goes through Emotion, which both report |
| `divergences.json` | Where the ports and the recordings disagree by design: the plugins' findings on the authored files of other JSX libraries |
| `licenses/<project>/` | The upstream licence and `UPSTREAM.md`, listing every copied file and extracted case |
| `candidates/` | Ports still being evaluated, scored but never shipped: the code-rule documents (`rules/`, written by `build.mjs` from `specs.mjs`, except the hand-written `react-no-array-index-key.json`); `probes/` holds the hand-labelled React fact probes |

Re-recording runs offline against the pinned tools (eslint 9.39.5, @typescript-eslint/parser 8.70.1, typescript
5.9.3) and the plugins' upstream checkouts, installed under `.rerail/tools/react`, with the corpus checkout under
`.rerail/corpora/react`:

```sh
node upstream/eslint-plugin-react/generate.mjs [--tools <dir>] [--corpora <dir>] [--no-corpora]
node upstream/eslint-plugin-react-hooks/generate.mjs [--tools <dir>] [--corpora <dir>] [--no-corpora]
```

Rewriting the rule documents and `conformance.json` after editing `candidates/specs.mjs`, then scoring one rule with
every false positive and false negative printed:

```sh
node candidates/build.mjs
node --import tsx conformance/languages/cli.ts --pack react --rule react-jsx-key --fixtures
```
