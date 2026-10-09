# Rerail rule pack: TSLint

Ports of TSLint core rules to `rerail.policy/v1`, each with TSLint's default options. One document per rule,
`rules/<rule>.json`, rule id `tslint:<rule>`, violation code `<rule>`. Core ships this pack inside its own package
until it loads installed packs, and seeds the rules into the rule catalog **disabled**; enable the ones you want. Every
document has `mode: "steer"`: an enabled rule steers the agent. None uses `block` until block enforcement lands.

Each rule's `scope` covers TypeScript sources (`.ts`, `.tsx`, `.mts`, `.cts`) and leaves out dependency, build-output
and generated directories (`node_modules`, `dist`, `build`, `out`, `.next`, `coverage`, `generated`, ...).

The pack follows the [pack structure and validation contract](../../docs/pack-format.md) like every other pack, and
its ports are scored the same way: against TSLint's own rule tests, recorded in `upstream/tslint/`.

```sh
npx vitest run test/engine/catalog/packs.test.ts -t "pack tslint"                          # checks 1 to 6
node --import tsx conformance/languages/cli.ts --pack tslint --fixtures      # every port's TP/FP/FN
node --import tsx conformance/languages/cli.ts --pack tslint --rule quotemark --rule quotemark--single
npx vitest run test/conformance/identifier-rename.test.ts                   # the identifier-rename gate
```

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group |
| `rules/` | The shipped ports |
| `conformance.json` | Every port (`rules/` and `candidates/`): the TSLint rule it reproduces and the tests it is scored on |
| `divergences.json` | Where a port and TSLint's tests are declared not comparable: test lines that are not valid TypeScript |
| `upstream/tslint/` | TSLint's rule tests (`fixtures/`), what they expect (`fixtures.expected.json`), the names the rename gate keeps (`protected-identifiers.json`) and the recorder (`generate.mjs`) |
| `candidates/<rule>/` | Ports not shipped: the default-options port (`<rule>.json`), option-variant ports (`<variant>.json`, shipped rules' too) and `verdict.json`, why the rule is not shipped |
| `licenses/tslint/` | TSLint's licence and `UPSTREAM.md` |

## How TSLint's tests are scored

A TSLint rule test is a directory with a `tslint.json`, a *variant* (the rule with one set of options), and `.lint`
files whose `~~~ [message]` marks are the failures TSLint must report. `upstream/tslint/generate.mjs` records them
offline at TSLint 6.1.3 (commit `285fc1db18d1fd24680d6a2282c6445abf1566ee`, the archived repository's last master),
evaluating `[typescript]` gates and `#if typescript` blocks against TypeScript 4.0.2, TSLint's last pinned version.
Nothing fetches TSLint at test time.

The code-rule DSL has no rule options, so each variant is scored with its own document: `conformance.json` lists one
port per document, and its `fixtures` globs select the variants it runs on. A rule's default-options port
(`rules/<rule>.json`, or `candidates/<rule>/<rule>.json` until shipped; port id `<rule>`) covers every variant without
an override; `candidates/<rule>/<variant>.json` (port id `<rule>--<variant>`, document id `tslint:<rule>/<variant>`)
covers its variant. Findings match on line and column.

Shipped ports are gated: `npm run test:conformance` and `test/engine/catalog/packs.test.ts` fail on any unexplained false positive
or false negative. Candidates are scored and reported, never gated. A few TSLint tests contain lines that are not valid
TypeScript; TSLint's parser recovers, the grammar does not, and every finding that costs is declared in
`divergences.json` with the invalid line.

The identifier-rename gate scores every shipped port again on the tests with each user-chosen identifier renamed to an
opaque one of the same length and shape: a port that matches names only the tests use (`foo`, `MyClass`) instead of
structure changes score and fails it. Names a rule may legitimately match are kept: the language's keywords and
standard library, a host list, the identifier-like literals in TSLint's implementation of the rule and the words of
each test's options (`protected-identifiers.json`).

## Results

Of TSLint's 163 core rules, 14 pure formatter rules are out of scope (align, eofline, indent, jsdoc-format,
max-line-length, no-consecutive-blank-lines, no-trailing-whitespace, one-line, semicolon,
space-before-function-paren, space-within-parens, trailing-comma, typedef-whitespace, whitespace): layout belongs to
a formatter. Each of the other 149 has a port, scored against TSLint 6.1.3's tests on kernel 1: 1,815 true
positives, 62 false positives and 794 false negatives in all. 113 rules match every test, 18 of them once the invalid
test lines are discounted; the other 36 need what a syntactic engine lacks (23 need TypeScript's type checker, file
bytes or other files; 13 a missing primitive), as each `verdict.json` records.

75 rules ship. Each matches every TSLint test of its rule (935 true positives, 188 more on the lines declared not
comparable), scores identically on renamed tests, implements TSLint's semantics rather than the tests' (not fitted to
what the tests happen to miss, and not baking an option list where TSLint's default differs), and has meaningful
option-less behaviour in TSLint. The 38 other rules that pass are candidates for one of those reasons, which their
`verdict.json` (`audit`, `requiresConfiguration`) gives. Three shipped rules have no TSLint test with the default
options (`no-invalid-this`, `no-submodule-imports`, `triple-equals`): their option variants are scored, and their
examples in `pack.json` prove the shipped document.

## Adding a rule

1. Write the port as `candidates/<rule>/<rule>.json` (plus `candidates/<rule>/<variant>.json` for option variants),
   add its ports to `conformance.json` with the variants they cover, and run `generate.mjs` against a TSLint checkout
   to record the rule's tests.
2. Iterate with `node --import tsx conformance/languages/cli.ts --pack tslint --rule <rule> --fixtures` until it
   passes. Record in `candidates/<rule>/verdict.json` why it cannot ship if it cannot.
3. To ship it, move it to `rules/<rule>.json` with the shipped scope and a description that starts with
   ``Port of TSLint's `<rule>` rule with its default options (https://palantir.github.io/tslint/rules/<rule>/).``,
   point its `conformance.json` port at it, list it in `pack.json` with its `document`, `title` and `group`, and add a
   changeset. The conformance and rename gates then hold it.

## Attribution

The rule semantics, names and descriptions come from [TSLint](https://github.com/palantir/tslint)
(Copyright 2013 Palantir Technologies, Inc.), licensed under the
[Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0). These documents are Rerail's
re-implementations in its policy DSL; TSLint is archived and not a dependency. `upstream/tslint/fixtures/` holds
TSLint's own rule tests with their markup removed. TSLint's licence and the record of what is copied or adapted from it
are in [`licenses/tslint/`](licenses/tslint/UPSTREAM.md), and the notice is also in core's
[`THIRD_PARTY_NOTICES`](../../THIRD_PARTY_NOTICES).
