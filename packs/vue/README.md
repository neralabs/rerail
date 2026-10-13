# Rerail rule pack: Vue

24 code rules for Vue single-file components, ported from eslint-plugin-vue 10.11.1 with its default options. Each
reads the component's tree-sitter-vue tree and the ECMAScript program its scripts and template expressions make, and
is scored against eslint-plugin-vue's own findings on its RuleTester cases and on three pinned corpora (element-plus,
vue-vben-admin, hoppscotch), through the engine's production workspace path. Every port agrees with eslint-plugin-vue
on every case and corpus file: 999 findings on the fixtures and 221 on the corpora, with no false positive or false
negative, and `divergences.json` declares none.

It is checked by:

```sh
node --import tsx conformance/languages/cli.ts --pack vue
npx vitest run test/engine/catalog/packs.test.ts -t "pack vue"
```

([pack structure and validation contract](../../docs/pack-format.md)).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream rule, how findings compare, and the capabilities it exercises; the pinned parser stack and the pinned corpora |
| `divergences.json` | Declared disagreements with eslint-plugin-vue (none) |
| `upstream/eslint-plugin-vue/` | The copied RuleTester cases (`fixtures/`), each case's fate (`cases.json`) and the recordings (`fixtures.expected.json`, `corpus-<name>.expected.json`) |
| `licenses/eslint-plugin-vue/` | eslint-plugin-vue's licence and `UPSTREAM.md`, listing every copied file |
