# Notes: batch javascript-01 (accessor-pairs, default-case, id-length, max-classes-per-file, max-statements, no-duplicate-imports)

Ground truth is ESLint 10.11.0 (the version `packs/javascript/pack.json` covers and the fixtures' recording was made
with), run with `npx eslint --no-config-lookup --rule …` on a fleet box, typescript-eslint's parser for `.ts`.
Where ESLint 9.39 differs it is named. Every count below is from `translations:check` (ptcheck) on the integration
branch plus this batch's files.

## Engine gap: `earlier` has no rows here

`earlier(p, x)` gave no rows for two import statements of one `.js` file under `run-policies` (a debug clause
`kind(x, "import_statement"), kind(p, "import_statement"), earlier(p, x)` reported nothing; the same with
`node(d, p, _, s, _), node(d, x, _, t, _), s < t` reported both files). NOTES-misc-untranslated.md records the same
for `visit_index`. no-duplicate-imports and default-case used it and lost every finding that needed it; both now
compare start offsets from `node`, which is what core says `earlier` is outside Python. No library change needed;
the engine should give `earlier`/`visit_index` rows for TypeScript and JavaScript.

## eslint-no-duplicate-imports: same

Was: no finding at all (fixtures 19, rerail-typescript 28 original-only), because `dup` ordered the two declarations
with `earlier`. `import "fs"; import "fs"` is reported by ESLint at line 2; now by the translation too.

## eslint-default-case: same

Was: fixture `default-case/019.js` original-only. `switch (a) { case 1: break; // no default⏎ // nope⏎ }`: ESLint
reads only the last comment (`nope`), so it reports. The translation's "no later comment" test used `earlier`,
which held for no pair, so every comment before the `}` counted as the last one and `no default` excused the switch.
Now the later comment is found by start offset.

## eslint-accessor-pairs: same

Was: `/* globals Object:off */ Object.defineProperty(foo, 'bar', { set(value) {} })` (fixtures
`accessor-pairs/067.js`, `no-setter-return/102.js`) translation-only. ESLint 10.11 asks
`sourceCode.isGlobalReference(callee.object)`: the name must resolve to the global variable, and the comment
removes it, so nothing is reported. (ESLint 9.39 does not check and reports both; the pack follows 10.11.) The
translation followed 9.39's "no local declaration" (`!ref_site`); it now asks eslint-b-scope's `builtin_ref`, which
also honours `/* global x:off */`.

## eslint-max-statements: same

Was: `src/hosts/omp/extension.ts` 418 (translation) against 422 (original). `const ingestEvent = async (⏎ … ⏎ ):
Promise<…> => {`: ESLint reports at `astUtils.getFunctionHeadLoc`, the arrow's `=>` (422:44). The translation
reported at the function's start (`fn_start`); it now uses `fn_head` (an arrow at `=>`, a method or property value
at its holder), as eslint-a-functions provides.

## eslint-id-length: same

Fixed, each checked against ESLint 10.11 on the fixture and on ad-hoc cases:

- **Overloads and signatures** (func-style 058–062, 071–075, max-params 024/025, no-dupe-class-members 068,
  rerail-typescript `test/support/ui-modules.d.ts`): `function test(a: string): string;` and `declare function
  mk(m: number): Date;` are TSDeclareFunction, a class method overload has a TSEmptyBodyFunctionExpression, an
  interface method or call signature and a function type are TS* nodes; id-length's parent table has none, so their
  parameters are not checked. The translation checked every `formal_parameters`; now only those of a function with a
  body (`body_params`).
- **Parameter properties with a default** (default-param-last 062–065, 092, 094): `constructor(private c = 10)` is a
  TSParameterProperty around an AssignmentPattern, whose `left` id-length checks (`public a: number` alone is not
  checked). Added.
- **An arrow's expression body** (no-return-assign 021/022, one-var 210): id-length checks every Identifier whose
  parent is an ArrowFunctionExpression, so `() => a` reports `a`. Added (parentheses skipped, as ESTree has none).
- **Escaped names** (no-nonoctal-decimal-escape 002, no-useless-rename 058/059/094/106/107): ESTree's name is
  decoded: `var \u0061` is `a` (too short); `let {a: \u0061} = o` has the same key and value, so the key is the
  checked one; `import {\u0061 as a}` is not a rename and is not checked. Names now come from eslint-b-objects'
  `id_value` (ASCII escapes decoded); an escape it cannot decode (`\u8888`) counts as one character.
- **Graphemes** (id-length 069, `var 葛󠄀`: a base and a variation selector): ESLint counts graphemes. Length now
  drops combining marks (`\p{M}`), ZWNJ and ZWJ before counting, as the JSON's regex does. Hangul jamo sequences
  (L V T as separate code points) would still count as several; no fixture has one.

Known gap, shared by both sides on the fleet's engine:

- **U+2028 line numbering** (no-irregular-whitespace 181): `var a = 'b',<U+2028>c = 'd',⏎e = 'f'<U+2028>`. JavaScript
  counts U+2028/U+2029 as line terminators, and ESLint reports `a` 1, `c` 2, `e` 3. The engine's `line` counts only
  `\n`: the translation reports lines 1, 1, 2, and so does the original on the fleet's build, so the suite says
  `same`. The macOS build of the original (ptcheck's local slot) counts them, and there the line-3 key is original-only.
  A rule cannot move a finding's line; the fix is the engine's `line` for JavaScript and TypeScript sources (as
  NOTES-js-1 records for no-irregular-whitespace).

## eslint-max-classes-per-file: different, translation right

`max-classes-per-file/004.js`: `/* comment */⏎/* eslint-disable rule-to-test/max-classes-per-file */⏎class A {}⏎class
B {}`. ESLint 10.11 reports `3:1 File has too many classes (2)` (and that `rule-to-test/max-classes-per-file` is not
defined): the directive names another rule. The original's run suppresses it: its oxlint suppression matches a
directive by the bare rule name after the last `/` (`native/src/code_rule_dsl/suppress/oxlint.rs`), so
`rule-to-test/max-classes-per-file` silences `max-classes-per-file`. The translation declares the same suppressions
and is not suppressed under `run-policies`. Nothing in the rule decides it; the two runs' suppression handling
differs.

## Library changes

None needed. No new library file: the shared concept (ordering by offset) is one atom, not vocabulary.
