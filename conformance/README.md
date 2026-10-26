# Conformance

Test support that scores rule packs against the tools they port. It is never published (it is outside package.json
`files`), and `src/` never imports it.

- `pack-validation.ts`: the pack validation contract, checks 1 to 8 of [docs/pack-format.md](../docs/pack-format.md).
  `test/engine/catalog/packs.test.ts` runs it for every `packs/<id>/`.
- `languages/`: the common conformance runner. It scores each ported rule TP/FP/FN through the production workspace,
  against the pack's upstream recording (`packs/<id>/upstream/<tool>/`, pack test data that `npm run fixtures:pull`
  fetches from the archive the tool-pack's `test-data.lock` names) and its declared divergences. The CLI
  is `npm run conformance:languages`, with `-- --check` as CI's gate (`npm run test:conformance`).
  `identifier-rename.ts` is the overfitting guard: a port must score the same when the tests' own identifiers are
  renamed.
  For the javascript and typescript packs it also scores the source `eslint-test-set`: 1,400 real files from
  open-source history, each at the commit that introduced a violation of one ESLint rule, where ESLint's findings for
  that rule are expected and any other finding of it is an FP, and the 1,384 copies with that rule's findings fixed by
  hand, where none is. Each file is evaluated for its own rule alone. `--check` scores it (a shipped rule fails on an
  unexplained FP or FN there, as on the fixtures), and `-- --eslint-test-set` scores it alone. The zip is pinned by its
  sha256 (`ESLINT_TEST_SET` in `scripts/s3/pack-fixtures.mjs`) at
  `s3://nera-test-fixtures/rerail/archives/eslint-test-set/<sha256>.zip`; `npm run fixtures:pull` fetches it with those
  packs' archives, and the runner fetches it itself when it is not in `.rerail/pack-data/archives/`, verifies it and
  extracts it into `.rerail/pack-data/eslint-test-set/`.
- `recorders/`: the shared ESLint recorder the ECMA packs' upstream generators use.
- `t1/`: the TypeScript grammar census (tier T1 of the language conformance standard).
- `engine.ts`: what every harness shares about the engine it scores (the capabilities, and which engine failure codes
  are not failures).

Adding a pack or a port: [docs/language-packs.md](../docs/language-packs.md#start-a-pack).
