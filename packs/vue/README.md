# Rerail rule pack: Vue

Ports of 24 eslint-plugin-vue 10.11.1 rules for Vue single-file components, with their default options. All 24 are
candidates under `candidates/rules/`: they have not passed the V1 rule validation gates
([nera-rule-validation](https://github.com/neralabs/nera-rule-validation)), so `pack.json` lists no rules, the pack
ships none and is never recommended, and a candidate is scored and reported but never gates a check. A port moves to
`rules/` and `pack.json` once it passes the gates. Each
reads the component's two documents: its tree-sitter-vue tree, and the ECMAScript program its scripts and template
expressions make (the `script-block` program a clause reads with `ast.embedded`, or reaches from a template node with
`embedded`; `host` leads back from a program node to the template node it stands for). Each port
is scored against eslint-plugin-vue's own findings on its RuleTester cases, on cases Rerail wrote for inputs a port once
misread (`fixtures/<rule>/rerail-*.vue`, listed in `upstream/eslint-plugin-vue/UPSTREAM.md`) and on three pinned corpora
(element-plus, vue-vben-admin, hoppscotch), through the engine's production workspace path. Every port agrees with
eslint-plugin-vue on every fixture case: 1,106 findings on the 773 fixture files, with no false positive or false
negative and no declared divergence. On the corpora (2,111 files) the ports agreed on all 221 findings when last scored,
before they were ported to the two documents.

It is checked by:

```sh
node --import tsx conformance/languages/cli.ts --pack vue
npx vitest run test/engine/catalog/packs.test.ts -t "pack vue"
```

([pack structure and validation contract](../../docs/pack-format.md)).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): its match definition, and no rules until a port passes |
| `candidates/rules/` | The 24 candidate code-rule documents (`rerail.policy/v1`), scored but never shipped |
| `conformance.json` | Each port's upstream rule, how findings compare, and the capabilities it exercises; the pinned parser stack and the pinned corpora |
| `divergences.json` | Declared disagreements with eslint-plugin-vue (none) |
| `upstream/eslint-plugin-vue/` | The copied RuleTester cases (`fixtures/`), the cases Rerail wrote (`authored.mjs`, written as `fixtures/<rule>/rerail-*.vue` and listed in `UPSTREAM.md`), each case's fate (`cases.json`) and the recordings (`fixtures.expected.json`, `corpus-<name>.expected.json`) |
| `licenses/eslint-plugin-vue/` | eslint-plugin-vue's licence and `UPSTREAM.md`, listing every copied file |
