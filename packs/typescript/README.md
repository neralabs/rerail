# Rerail rule pack: TypeScript

25 code rules for TypeScript, ported from typescript-eslint 8.70.1 (on ESLint 10.11.0), each with its default
options. 11 start off (`"on": false`): no approved typescript-eslint configuration turns them on, so they are
experimental. Each is scored against the tool's own findings on the tool's test fixtures and on a pinned corpus
(vscode), through the engine's production workspace path, with every disagreement declared in `divergences.json`: 6
today, all `rerail-unknown`, each a file the tree-sitter grammars read with a syntax error. ESLint's core rules are in
the [JavaScript pack](../javascript/README.md).

It is checked by `npx vitest run test/engine/catalog/packs.test.ts -t "pack typescript"`, against the
[pack structure and validation contract](../../docs/pack-format.md).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title, group and, for a rule that starts off, `on`; two rules carry examples |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools and the pinned corpus (vscode) |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`) and its recordings (`fixtures.expected.json`, `corpus-vscode.expected.json`) |
| `upstream/generate.mjs` | typescript-eslint's recorder; `upstream/eslint-plugin-vitest/generate.mjs` records @vitest/eslint-plugin |
| `candidates/rules/` | Ports still being evaluated, never shipped: typescript-eslint and @vitest/eslint-plugin 1.6.27 ports that fail a V1 validation gate or rerail's pack contract. Each but `prefer-optional-chain` (it requires type information, which the recordings do not have) and `no-explicit-any` (the fixtures recording does not cover it yet) is listed in `conformance.json`, scored and reported on the fixtures, never gated; the vscode recording covers the shipped rules only |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, listing every copied file |

Re-recording runs offline against the pinned tool versions (the recorder's header lists them):

```sh
node upstream/generate.mjs --tools <dir> --typescript-eslint <typescript-eslint checkout at v8.70.1> \
  --fixtures --corpus vscode=<vscode checkout>
node upstream/eslint-plugin-vitest/generate.mjs --tools <dir> --vitest <eslint-plugin-vitest checkout at v1.6.27>
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack typescript --rule typescript-eslint-no-namespace --fixtures
```

## Not ported: `no-restricted-types`

Every typescript-eslint 8.70.1 rule that needs no type information has a port here, shipped or candidate, except
`no-restricted-types`. It reports only the types a project lists in its `types` option, and that list is empty by
default. At its default options it reports nothing, so there is nothing to port. A project that wants a type banned
writes its own code rule.

## `no-magic-numbers` passes the gates and stays a candidate

It passes Gates 2 to 5 and agrees with typescript-eslint on every fixture and every vscode file. A shipped rule's vscode
recording has to cover it, though, and at its default options it reports nearly every number literal: about 187,500
findings in vscode's `src/`. That would grow `corpus-vscode.expected.json` from 4.5 MB to about 41 MB, so the rule
stays in `candidates/` until the recording can hold it. It would start off anyway: no approved typescript-eslint
configuration turns it on.

## `init-declarations` can't pass Gate 5

typescript-eslint declares `'always'` as `init-declarations`' default. Its wrapper around ESLint's core rule has no
`meta.defaultOptions`, so with no options ESLint 10 passes the core rule no mode, and it reports nothing. Every upstream
test that expects a finding sets the option, so upstream's own tests never notice. Gate 5 runs each case at the rule's
default options, sees no upstream finding at all, and fails the rule whatever its document says. The candidate ports the
declared default, `'always'`. With that option set, it agrees with typescript-eslint on every upstream test case and on
the vscode corpus.
