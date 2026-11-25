# Batch misc-untranslated: javascript

## eslint-one-var: different, the translation right

Three defects in revision 4, each fixed:

1. check-rr refused it: `` `ordered` on .decl crowded `` is not lowered. `crowded` read itself under `!` through
   earlier declarations. It is now unrolled into three rounds of claims (after ce248cd129's version): the claims of a
   scope and kind only grow, over three classes (uninitialized, initialized, require), so the declarations that pass
   and record classes are found round by round, each claiming a class more.
2. It reported nothing at all. `!option(_, _)` ("no options: always") never held: the engine adds options of its
   own (`dialect`, `reuse`) to every run. The default now holds when none of the rule's own option names is set.
3. "First" and "after" read `visit_index`, which has rows for Python only. They now compare `node` start offsets.

Remaining difference, rerail-typescript, 130 translation-only, 0 original-only. ESLint 9 `one-var` (default
options, typescript-eslint parser, `--no-ignore`) over the whole corpus reports every one of the 130:

- 126 in `src/engine/code-rule/code-rule-schema.ts`: the original spends its work budget there (`budget-exceeded`)
  after one finding (line 347). ESLint reports 127 lines in the file, the translation the same 127.
- 4 in `test/support/ui-modules.d.ts` (132, 203, 276, 308): a later `export const` in a `declare module "…" { … }`
  block, after an earlier one in that block. ESLint: "Combine this with the previous 'const' statement."; the
  original misses them.

ESLint reports 4 more keys that neither side has (`test/support/build/script-bundle.ts:9, 32, 35`,
`test/scripts/build/pull-engine.test.mjs:28`): files under a `build/` directory, which the corpus scan skips for
both sides. Run on the files alone, the translation reports them.

The `!option(_, _)` spelling is also recorded in NOTES-statements.md; any rule that copied it has the same defect.

Library changes: none needed. Engine gap: `visit_index`/`earlier` for TypeScript (the comment in core says "elsewhere
it is start offset", but no rows came back for `visit_index` on a `.js` file).
