# Pack validator fixtures

Test inputs of the pack validator (`conformance/pack-validation.ts`), run by `test/packs.test.ts`. Each directory is a
deliberately small pack in the [pack format](../../../docs/pack-format.md), not a rule pack Rerail ships:

- `valid/` passes every check: one example-proven rule, and one ported rule scored on its upstream recording whose
  tool records source facts (`upstream/examplelint/source.json`), so checks 7 to 9 judge it. Each negative test copies
  it and breaks one sub-check.
- `vue/` is a pack for a language core plans but does not read yet: it passes, its rule reported as not supported yet.

The rules here exist only to exercise the validator. No package's `files` includes this directory, and nothing here is
published or installed.
