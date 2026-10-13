# Pack-shaped test inputs

Small packs in the [pack format](../../docs/pack-format.md) that tests read, not rule packs Rerail ships:

- `valid/` passes every check of the pack validator (`conformance/pack-validation.ts`, run by `test/packs.test.ts`):
  one example-proven rule and one ported rule scored on its upstream recording, whose tool fixtures hold one copied
  file (`licenses/examplelint/UPSTREAM.md`) and one Rerail wrote (`upstream/examplelint/UPSTREAM.md`). Each negative
  test copies it and breaks one check.
- `vue/` is a pack for a language core plans but does not read yet: it passes, its rule reported as not supported yet.
- `empty/` ships no rules, as a pack does while all its ports are candidates: it passes, with no rules to report.
- `sample-php/` is the conformance runner's sample (`conformance/languages/runner.ts`, run by
  `test/language-conformance.test.ts`): two ports scored against PHP_CodeSniffer's recordings on authored fixtures and
  a one-file corpus (`corpora/mini/`).

The rules here exist only to exercise the validator and the runner. No package's `files` includes this directory, and
nothing here is published or installed.
