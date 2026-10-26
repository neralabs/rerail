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
  It also scores each rule on the pack's real-code samples (`--samples`; `--check` scores the fixtures and the
  samples): real files from open-source repositories, each labelled, per pack rule, with the lines the rule should
  report or as clean for it (`scripts/samples/format.ts`). A tool-pack's samples are one archive in S3 that its tracked
  `upstream/<tool>/samples.lock` names by sha256, beside a `SAMPLES.md` listing the source repositories, commits and
  licences; `npm run fixtures:pull` (or `node scripts/s3/pack-fixtures.mjs samples`) fetches, verifies and extracts it
  under `.rerail/pack-data/samples/`. Judgement is by line and by intent: a report on a labelled line is a TP, a
  labelled line with no report a miss, any report on a sample clean for the rule an FP, and an extra report on a
  sample with labels for the rule is listed for review only. A shipped rule fails on an unexplained miss or FP and on
  an engine failure there; a parse failure on a sample (a syntax error, a partial or incomplete source) is report-only
  and the sample stays judged. Candidates are reported only. See
  [docs/port-readiness.md](../docs/port-readiness.md#samples).
- `recorders/`: the shared ESLint recorder the ECMA packs' upstream generators use.
- `t1/`: the TypeScript grammar census (tier T1 of the language conformance standard).
- `engine.ts`: what every harness shares about the engine it scores (the capabilities, and which engine failure codes
  are not failures).

Adding a pack or a port: [docs/language-packs.md](../docs/language-packs.md#start-a-pack).
