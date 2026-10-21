# Conformance

Test support that scores rule packs against the tools they port. It is never published (it is outside package.json
`files`), and `src/` never imports it.

- `pack-validation.ts`: the pack validation contract, checks 1 to 8 of [docs/pack-format.md](../docs/pack-format.md).
  `test/engine/catalog/packs.test.ts` runs it for every `packs/<id>/`.
- `languages/`: the common conformance runner. It scores each ported rule TP/FP/FN through the production workspace,
  against the pack's upstream recording (`packs/<id>/upstream/<tool>/`) and its declared divergences. The CLI is
  `npm run conformance:languages`, with `-- --check` as CI's gate (`npm run test:conformance`).
  `identifier-rename.ts` is the overfitting guard: a port must score the same when the tests' own identifiers are
  renamed.
- `eslint-test-set/`: checks the javascript and typescript packs catch every ESLint finding in the ESLint test set, 1,400
  real files from open-source history, each at the commit that introduced a violation of one rule. It downloads the
  pinned zip from `s3://nera-test-fixtures/eslint-test-set/` (AWS credentials needed; `--profile <name>` picks a profile)
  and caches it in `.rerail/cache/`. The CLI is `npm run eslint-test-set`; `-- --rule <id>` checks one rule,
  `-- --json <file>` writes the report. CI does not run it: it needs AWS credentials, and its misses are not yet triaged.
- `recorders/`: the shared ESLint recorder the ECMA packs' upstream generators use.
- `t1/`: the TypeScript grammar census (tier T1 of the language conformance standard).
- `engine.ts`: what every harness shares about the engine it scores (the capabilities, and which engine failure codes
  are not failures).

Adding a pack or a port: [docs/language-packs.md](../docs/language-packs.md#start-a-pack).
