# Pack-shaped test inputs

Small packs in the [pack format](../../docs/pack-format.md) that tests read, not rule packs Rerail ships:

- `valid/` passes every check of the pack validator (`conformance/pack-validation.ts`, run by `test/packs.test.ts`):
  one example-proven rule and one ported rule scored on its upstream recording, whose tool fixtures hold one copied
  file (`licenses/examplelint/UPSTREAM.md`) and one Rerail wrote (`upstream/examplelint/UPSTREAM.md`). Each negative
  test copies it and breaks one check.
- `exemplar/` is a pack for the test-only planned language ([`exemplar.json`](../../native/test-languages/exemplar.json)): it passes, its rule reported as not supported yet.
- `empty/` ships no rules, as a pack does while all its ports are candidates: it passes, with no rules to report.
- `eslint-port/` ports one ESLint rule whose violation carries `suppress.eslint`, scored on a file Rerail wrote with
  `eslint-disable` comments: it passes, and copies of it prove the suppression check of check 4 (a port without
  `suppress`, an exempted one, a stale exemption).
- `config-regex/` ships one rule whose `pattern` option is written into a regex: the rule settings API refuses a
  value the option allows but the engine cannot compile (`test/api/api-rule-settings.test.ts`).
- `rr-sample/` ships three rules written in `.rerail`: `rr-sample-assert` is one file, `rr-sample-print-call`
  includes the pack's own library file, `rules/lib/sample-calls.rerail`, and `rr-sample-deprecated-call` reports a
  call for what another file says of the function it calls. It passes every check, and copies of it prove what the
  catalog and checks 1, 2, 3, 5 and 8 refuse in a `.rerail` rule (`test/engine/catalog/packs.test.ts`,
  `test/engine/catalog/rerail-rules/rerail-pack-rules.test.ts`); sessions and audits run it
  (`integration/rerail-pack-rules.test.ts`).
- `rr-curly/` ships `eslint-curly`, the translation of ESLint's curly written in `.rerail` as the translations write
  it, with the options it reads, its fixes and its ESLint, Oxlint and Biome `suppress` lines, and a copy of the translations'
  `tsx/statements.rerail` as the pack's own library file. It proves a `.rerail` rule's options, fixes and suppression
  comments through the catalog, settings, set-up's lint config, the runner and check 2
  (`test/engine/catalog/rerail-rules/rerail-rule-options-fixes.test.ts`, decision 0720).
- `sample-php/` is the conformance runner's sample (`conformance/languages/runner.ts`, run by
  `test/engine/code-rule/language-conformance.test.ts`): two ports scored against PHP_CodeSniffer's expected findings on
  authored fixtures, and on real-code samples the test writes beside it.

The rules here exist only to exercise the validator and the runner. No package's `files` includes this directory, and
nothing here is published or installed.
