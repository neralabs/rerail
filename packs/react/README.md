# Rerail rule pack: React

No rules yet: the pack ships none and is never recommended while `pack.json` lists none. Its 21 ports of
eslint-plugin-react 7.37.5 and eslint-plugin-react-hooks 7.1.1 rules are candidates
([candidates/README.md](candidates/README.md)). A port starts in `candidates/rules/` and moves to `rules/` and
`pack.json` once it passes the conformance check ([conformance/README.md](../../conformance/README.md#the-gate)), which
CI runs on every port. Every port is scored against the plugins' own findings on the
plugins' test cases and on their rename variants (`holds-under-rename`), and against the labelled lines of the plugins'
real-code samples ([docs/upstream.md](../../docs/upstream.md#samples)), through the engine's production workspace
path. The conformance check (`--check`, run in CI) scores the test cases and the samples: it gates the ports in
`rules/` and reports the candidates without gating them.

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
| `conformance.json` | Every port (shipped under `rules/`, or a candidate under `candidates/rules/`): its upstream tool and rule, how findings compare and the capabilities it exercises; the pinned tools. Written by `candidates/build.mjs` |
| `upstream/<tool>/` | The plugin's copied tests (`tests/`), the cases extracted from them (`fixtures/`, their rename variants under `fixtures/renamed/`), its expected findings (`fixtures.expected.json`), the generator (`generate.mjs`, sharing `upstream/common.mjs` and `scripts/pack-data/eslint-expected.mjs`), and the lock naming its real-code samples (`samples.lock`, fetched by `npm run fixtures:pull`) with their sources and licences (`SAMPLES.md`); `UPSTREAM.md` lists the fixtures Rerail wrote beside them (`common.mjs` `AUTHORED`): files of other JSX libraries, which the plugins report and the ports skip, and React files whose JSX goes through Emotion, which both report |
| `divergences.json` | Where the ports and the expected findings disagree by design: the plugins' findings on the authored files of other JSX libraries |
| `licenses/<project>/` | The upstream licence and `UPSTREAM.md`, per upstream repository: its source, the patterns of the files taken and how (each file's provenance is `provenance.json` in the tool's test-data archive) |
| `candidates/` | Ports still being evaluated, scored but never shipped: the code-rule documents (`rules/`, written by `build.mjs` from `specs.mjs`, except the hand-written `react-no-array-index-key.json`); `probes/` holds the hand-labelled React fact probes |
| `candidates/README.md` | What each candidate is waiting on and its rule details; not published |

Taking a new snapshot is a maintainer deriving the pack's test data from its pins and publishing it
([docs/upstream.md](../../docs/upstream.md#take-a-new-snapshot)); a pull request carries it only as a publish request
under `upstream/<tool>/publish/`. The recipe (`scripts/pack-data/recipes/react.ts`) runs each plugin's `generate.mjs`
against the pinned tools (eslint 9.39.5, @typescript-eslint/parser 8.70.1, typescript 5.9.3) and the plugins' upstream
checkouts. The fixtures Rerail wrote (`UPSTREAM.md`) are authored: a change to them is staged into `publish/` with the
derived data.

Paused: `fixtures:derive` refuses every tool-pack until the generators write per-repo provenance
([docs/upstream.md](../../docs/upstream.md#derive-and-bumps-are-paused)). The commands below take a snapshot once the
pause lifts.

```sh
npm run fixtures:derive -- --pack react                                     # both plugins
npm run fixtures:derive -- --pack react --tool eslint-plugin-react-hooks    # one
npm run fixtures:stage -- --pack react --tool eslint-plugin-react-hooks     # copy what changed into upstream/eslint-plugin-react-hooks/publish/
```

Rewriting the rule documents and `conformance.json` after editing `candidates/specs.mjs`, then scoring one rule with
every false positive and false negative printed:

```sh
node candidates/build.mjs
node --import tsx conformance/languages/cli.ts --pack react --rule react-jsx-key --fixtures
```
