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
- `sample-php/` is the conformance runner's sample (`conformance/languages/runner.ts`, run by
  `test/engine/code-rule/language-conformance.test.ts`): two ports scored against PHP_CodeSniffer's expected findings on
  authored fixtures, and on real-code samples the test writes beside it.

The rules here exist only to exercise the validator and the runner. No package's `files` includes this directory, and
nothing here is published or installed.
