# Rerail rule pack: Next.js

17 code rules for Next.js projects, ported from `@next/eslint-plugin-next` 16.3.8. Each is scored against the
plugin's own findings on the plugin's rule tests, through the engine's production workspace path; no disagreement is
declared today, so the pack has no `divergences.json`.

The plugin has 22 rules: the 17 here, 4 candidates (`candidates/rules/`), and `no-html-link-for-pages`, which is
[out of scope](#out-of-scope-no-html-link-for-pages).

The pack follows the [pack structure and validation contract](../../docs/pack-format.md): it is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack next"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group; six rules carry examples |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream rule, how findings compare, and the capabilities it exercises, the shipped rules first and then the candidates; the pinned plugin and the pinned corpora (shadcn-ui, next-examples, commerce) |
| `upstream/eslint-plugin-next/` | The plugin's copied rule tests (`tests/`) and the cases extracted from them (`fixtures/`), its recordings (`fixtures.expected.json`, `corpus-<name>.expected.json`, `fixtures.cases.json`) and the recorder (`generate.mjs`) |
| `candidates/` | Ports that cannot pass their fixtures yet or that fail the V1 rule validation gates (`rules/`; scored through `conformance.json`, never gating), the Next fact model's probes (`probes/`), and the generators that write them and `rules/` (`build-policies.mjs`, `build-probes.mjs`) |
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

Scoring one rule, shipped or candidate, with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack next --rule next-no-img-element --fixtures
```

## Out of scope: `no-html-link-for-pages`

The plugin's rule, on in its `recommended` config, reports an `<a>` whose `href` is the URL of one of the app's own
pages. It finds the pages when ESLint starts: it lists every `.js`, `.jsx`, `.ts` and `.tsx` file under `pages/`,
`src/pages/`, `app/` and `src/app/` (relative to the working directory, or to `settings.next.rootDir`) and turns each
into a URL pattern, where `index` is its directory and a `[param]` segment matches any segment. It then reports each
`<a>` whose literal `href`, not absolute and without `target="_blank"` or `download`, matches one of the patterns.

Rerail's code rules cannot express this. The engine does read every file of the workspace, but a clause reports a
witness for what its own file holds (`forbid`), for a correlated file that is missing (`require_each`), or for a query
that matches anywhere in the workspace, correlated with nothing (`select_when`). No form reports a witness because a
file matching one of its captures exists, which is the polarity this rule needs. Two smaller gaps sit behind it: an
evidence relation projects a file's path in pieces, never as a URL, and its `where` compares projected fields only
for equality, so a `[param]` route could not match.

The rule can be ported once the engine has a clause that reports a subject with matching evidence, and a comparison of
a captured `href` with a route pattern.
