# Conformance

Test support that scores rule packs against the tools they port. It is never published (it is outside package.json
`files`), and `src/` never imports it.

- `pack-validation.ts`: the pack validation contract, checks 1 to 8 of [docs/pack-format.md](../docs/pack-format.md).
  `test/engine/catalog/packs.test.ts` runs it for every `packs/<id>/`.
- `languages/`: the common conformance runner. It scores each ported rule TP/FP/FN through the production workspace,
  against the pack's upstream recording (`packs/<id>/upstream/<tool>/`, pack test data in S3 that
  `npm run fixtures:pull` fetches) and its declared divergences. The CLI is
  `npm run conformance:languages`, with `-- --check` as CI's gate (`npm run test:conformance`).
  `identifier-rename.ts` is the overfitting guard: a port must score the same when the tests' own identifiers are
  renamed.
- `recorders/`: the shared ESLint recorder the ECMA packs' upstream generators use.
- `t1/`: the TypeScript grammar census (tier T1 of the language conformance standard).
- `engine.ts`: what every harness shares about the engine it scores (the capabilities, and which engine failure codes
  are not failures).

Adding a pack or a port: [docs/language-packs.md](../docs/language-packs.md#start-a-pack).
