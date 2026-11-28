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

Library changes: none needed for the translation itself; the review fix below changes `lib/tsx/statements.rerail`.
Engine gap: `visit_index`/`earlier` for TypeScript (the comment in core says "elsewhere
it is start offset", but no rows came back for `visit_index` on a `.js` file).

### Review fix: `declare global { … }`

The review found `const x = 1; declare global { const y: number; }` unreported: ESLint parses the body as a
TSModuleBlock, which one-var's stack ignores, but tree-sitter puts a plain `statement_block` straight in the
`ambient_declaration`, so `block_scope_at` made it a scope. `namespace_body` (`lib/tsx/statements.rerail`, the one
definition of one-var's scopes) now also takes a `statement_block` child of an `ambient_declaration`.

| File | ESLint | Before | After |
|---|---|---|---|
| j1_declare_global.ts (review) | line 3 | nothing | line 3 |
| j2.ts: let/var in and around `declare global`, `global { }` in `declare module "m"` | 3, 4, 7, 11 | 4, 7, 11 | 3, 4, 7, 11 |
| j3.ts: `declare global` with an interface, blocks in a function | nothing | nothing | nothing |

`namespace_body` is also read by eslint-max-statements (`owned`), where ESTree's TSModuleBlock is no BlockStatement
either. ptcheck `--rules eslint-one-var,eslint-max-statements` after the fix: one-var unchanged (fixtures 311 both;
rerail-typescript 11026 both, 130 translation-only, the ones explained above); max-statements unchanged from the
integration branch without the fix (rerail-typescript 1401 both, `src/hosts/omp/extension.ts` 422 original-only and
418 translation-only on both runs).
