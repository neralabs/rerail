# NOTES: batch ts-1 (typescript-eslint, syntax only)

There are 33 rules. Each rule is checked against one of two references:

- **rrlab score.** `rrlab score typescript <id> --all` compares the rule with the typescript-eslint 8.70.1
  recording, across every fixture.
- **ad-hoc.** Seven rules have no recording ("cannot tell which upstream tool"). For those, the reference is the
  installed ESLint 9.39.3 with @typescript-eslint 8.56.1 (in `nera-landing-2/Data-layer-main-up/node_modules`),
  run on hand-written cases under several option sets. "matches" means the two agree on every reported
  line:column.

Each rule's revision is the JSON's revision plus 1. Its codes, message ids, suppressions, fixes and options
follow the JSON, except where noted below.

The rules use two libraries:

- `lib/tsx/ts-a.rr` holds expressions, comments, class and type members, and types.
- `lib/tsx/ts-a-returns.rr` holds the shared logic of the two return-type rules (typescript-eslint's
  explicitReturnTypeUtils).

| rule | lines | score (TP/FP/FN) | deviations |
|---|---:|---|---|
| adjacent-overload-signatures | 51 | 38/0/0 | |
| array-type | 108 | ad-hoc, 4 option sets match | |
| ban-ts-comment | 63 | 7/0/0, plus ad-hoc on 3 option sets | minimumDescriptionLength counts bytes (1) |
| ban-tslint-comment | 22 | 8/0/0 | |
| class-literal-property-style | 100 | 11/0/0 | `get [k]()` beside `set [k](v)` (2) |
| class-methods-use-this | 63 | 187/0/0 | `this` in a `this:` parameter (3) |
| consistent-generic-constructors | 61 | 19/0/1 | FN: fixture 036, an ASI misparse (4) |
| consistent-indexed-object-style | 129 | ad-hoc, both modes match | the JSON has no index-signature mode (5); `{ [K in X] }` (14) |
| consistent-type-assertions | 68 | ad-hoc, 5 option sets match | |
| consistent-type-definitions | 64 | 46/0/0 | |
| consistent-type-imports | 82 | 102/0/0, plus ad-hoc in both modes | no fix; names.rr `N.T` and shadowing (6) |
| default-param-last | 23 | 54/0/0 | |
| explicit-function-return-type | 49 | 406/0/0, also at column level | (7) |
| explicit-member-accessibility | 73 | 501/0/0, plus ad-hoc on 3 option sets | |
| explicit-module-boundary-types | 132 | ad-hoc, 2 option sets match, plus the reviewer's cases | names.rr scopes (8) |
| max-params | 46 | 8/0/0 | |
| member-ordering | 253 | 46/0/0, plus ad-hoc on 6 option sets | list entries of memberTypes (9) |
| method-signature-style | 56 | 209/0/0 | fix skipped for duplicate keys and readonly properties (10) |
| no-array-constructor | 24 | 16/0/0 | |
| no-confusing-non-null-assertion | 37 | 11/0/0 | |
| no-dupe-class-members | 43 | 12/0/0 | |
| no-duplicate-enum-values | 50 | 16/0/0 | |
| no-dynamic-delete | 26 | 10/0/0 | |
| no-empty-function | 63 | 428/0/0 | |
| no-empty-object-type | 65 | ad-hoc, 2 option sets match | the JSON has no options (5) |
| no-explicit-any | 41 | ad-hoc matches | |
| no-extra-non-null-assertion | 22 | 15/0/1 | FN: `x!!!`, two findings on one line:column (11) |
| no-extraneous-class | 57 | 106/0/0 | follows 8.70 on index signatures (12) |
| no-import-type-side-effects | 31 | 4/0/0 | adds the oxlint suppression |
| no-inferrable-types | 71 | 76/0/0 | |
| no-invalid-this | 43 | 78/0/0 | capIsConstructor=false not supported (13) |
| no-invalid-void-type | 111 | 39/0/0, plus ad-hoc on 3 option sets | |
| no-magic-numbers | 128 | ad-hoc, 3 option sets match | |

Totals:

- Rule lines: 2255.
- Library lines: 368 (`ts-a.rr` 231, `ts-a-returns.rr` 137).
- Scores: 24 of the 26 recorded rules are perfect. The other 2 each have one FN, explained below. All 7
  unrecorded rules match ESLint on every case tried.

## Explanations

1. **ban-ts-comment.** The plugin counts a description's length in graphemes. `strlen` counts bytes, so a
   non-ASCII description reaches the minimum sooner.

2. **class-literal-property-style.** A computed key with no static value pairs with a setter by its source text.
   8.70 does this: fixtures 014 and 015 are clean in the recording. 8.56 instead reports
   `get [k]() { return ''; }` beside `set [k](v) {}`.

   The JSON has no fixes. The plugin's suggestions are added as `to_field` and `to_getter`. The getter
   suggestion leaves the field's `;` after the new getter. That `;` is an empty class member, which is harmless.
   Like the plugin's `getStaticValue`, the rule does not resolve a `const` key through its variable.

3. **class-methods-use-this.** The rule excludes the `this` in a parameter type (`m(this: void)`). eslint-a-common's
   `this_owner` counts it (see the change requests).

4. **consistent-generic-constructors.** Fixture 036 is `new⏎ Foo⏎ ()`. tree-sitter reads it through ASI as two
   statements, so the constructor call is never seen.

5. **consistent-indexed-object-style and no-empty-object-type.** Their JSON has no options clause, and
   consistent-indexed-object-style's JSON has no index-signature mode. Both rules follow the tool and implement
   all of its options: `option` (`record` / `index-signature`), and `allowInterfaces`, `allowObjectTypes` and
   `allowWithName`.

   consistent-indexed-object-style's circularity check (isDeeplyReferencingType) resolves type names by name
   within the file. It also treats a mapped type's `: V` as no annotation, as TSESTree does.

6. **consistent-type-imports.**
   - No fix is offered.
   - An import that has attributes and only type uses is not reported.
   - The rule excludes the `T` in `N.T`. names.rr's `ref` reads that `T` as a reference (see the change requests).
   - `export type { B }` and `export { type B }` count as type uses, not value uses.
   - A value use that names.rr resolves both to a local (`function f(A) { g(A) }`, `let A`) and to the import
     belongs to the local. The rule drops it locally (`shadowed`); the cause is names.rr's `look` (see the change
     requests). This applies to value uses only: in `constructor(foo: foo.Foo)` the type's `foo` is the import.

7. **explicit-function-return-type.** An `accessor` field is an AccessorProperty in TSESTree, not a
   PropertyDefinition. So its type does not make the function typed, and its value is reported at the `=>`. That
   was the cause of the 2 FP / 2 FN at column level (class-methods-use-this 001 and 002); scoring is now perfect at
   column level too.
   - `new (function () {})()` is typed, as upstream's isConstructorArgument holds for the callee too.
   - allowIIFEs allows any function whose parent is a call, so `foo(() => 1)` is allowed too, as upstream.
   - A JSX spread attribute `<X {...f} />` is a typed place.
   - Harness gap: `export default function () { … }` followed by `(function () {})();` is one call expression in
     tree-sitter, so the second function is not reported (the reviewer's case e6, line 3).

8. **explicit-module-boundary-types.** `followReference` (on `export { a }`, `export default a`, and names inside
   exported arrays and objects) looks the name up in the module scope only, through names.rr's `decl_site`. A
   `var` in a top-level block counts. It then follows assignments that names.rr's `ref_site` resolves to that
   variable. A local of the same name inside a function shadows it: `function f() { let a; a = () => 1; }` is not
   followed (`local_too` works around names.rr's `look`). Imports, parameters and catch bindings are not followed.
   - Constructor and setter signatures (`constructor(a: string);`, `set x(v: number);`, `abstract set y(v)`) need
     no return type.
   - `(a: (any))` counts as `any`.

   Several upstream quirks are kept:
   - A `return` counts towards a higher-order function only when it sits directly in the function's body
     (`return.parent.parent`). So `if (x) return () => 1;` does not check the inner arrow.
   - An `export default function () {}` is a function_expression in tree-sitter.
   - The `allowExpressions` and `as const` branches of the shared utilities run only while
     allowTypedFunctionExpressions is on.

   Messages are the JSON's two texts, not the four parameterised ones.

9. **member-ordering.** A memberTypes entry that is itself a list (`[["field", "method"], "constructor"]`)
   reaches the rule as one string: rrlab's `option_rows` prints a nested list as its Python repr,
   `['field', 'method']`. The rule tests membership in that text (`'field'`). The engine needs a defined form for
   nested lists (see the change requests).

   Other choices:
   - The group-order message is the JSON's static text.
   - The alphabetical-order and optionality-order messages, which the JSON lacks, are the plugin's.
   - `natural` uses eslint-b-objects' `natural_key`. That library is uncommitted js-2 work, so this is a
     dependency to keep in mind.
   - The upstream quirk in groupMembersByType is reproduced: the body's last member is never sorted, and a
     member rejoins the previous group by rank.

10. **method-signature-style.** The plugin merges methods with the same key into one intersection fix. The rule
    reports each such method but does not fix any of them.

    On a readonly function property in "method" mode, the rule follows the JSON and does not fix. The plugin's
    fix would write `readonly m(): R`, which is not a valid member.

11. **no-extra-non-null-assertion.** `x!!!` has two findings at the same line:column. They collapse under the
    finding key, so one of them shows as an FN.

12. **no-extraneous-class.** The rule counts a non-static index signature as an instance member. This is the 8.70
    behaviour; 8.56 differs.

13. **no-invalid-this.** The rule is built on eslint-a-globals' `default_this`, plus a local `ts_bound`. `ts_bound`
    adds the two cases that `default_this` misses:
    - a capitalised TypeScript parameter default;
    - a JSDoc `@this` on an enclosing statement.

    capIsConstructor=false needs a switch in `default_this` (see the change requests).

14. **consistent-indexed-object-style.** `type L = { [K in X] };` is a syntax error to tree-sitter, so it is not
    reported. This is a grammar gap.

15. **Fix changes after review.**
    - consistent-type-assertions: `<T>(b || c)` becomes `((b || c) as T)`, from the expression as written. The
      annotation fix replaces the name (`x` → `x: T`) instead of using insert_after.
    - array-type: a readonly array that is another array's element keeps parentheses: `ReadonlyArray<string>[]`
      becomes `(readonly string[])[]`.
    - array-type and consistent-indexed-object-style skip `extends` and `implements` entries (ts-a `heritage`),
      as the JSON did.
    - explicit-member-accessibility: `drop_public` deletes the modifier node. A regex over the member could find
      `public` inside a decorator's string. The space after `public` stays.
    - consistent-type-definitions `to_type`: the body is rewritten in one `replace`, which carries the `= `, the
      `& A & B` heritage and the moved `export default`. Several insert_after edits on one node would be
      dropped.
    - no-import-type-side-effects: ` type` is added by replacing the `import` token, because insert_after takes
      nodes only.
    - member-ordering: a body kind counts as configured when it has `option_item` rows too (a list option
      comes only as those).
    - no-invalid-void-type: the climb for an overloaded implementation stops at function declarations, methods
      and `export default function`, not at other function expressions, as upstream.
    - class-literal-property-style: the fix sits on the finding's node, which for a computed key is the
      expression inside the brackets.

Other notes on specific rules:

- **no-magic-numbers.** In TypeScript places the TypeScript option alone decides, but the `ignore` list still
  wins. There, a `+` sign does not belong to the number: `enum E { A = +3 }` is reported at `3`, as "3". (The
  base rule's places keep the `+`.) An `accessor n = 5` is no class field for ignoreClassFieldInitialValues.
- **no-empty-function.** decoratedFunctions and overrideMethods apply to method_definition only.

## Library and core change requests

1. **eslint-a-common `this_owner`** counts the `this` of a `this: T` parameter as a use of `this`. It should
   skip a `this` that is a required_parameter's pattern.

2. **eslint-a-globals `default_this`** needs three changes:
   - Treat a TS parameter default with a capitalised pattern (required_parameter `value`) as a constructor
     context.
   - Climb to JSDoc on enclosing statements, as getJSDocComment does.
   - Add a capIsConstructor switch.

   ts-1's no-invalid-this carries the first two locally as `ts_bound`.

3. **names.rr `ref`** reads every type_identifier as a reference, including the `T` in `N.T`
   (nested_type_identifier).

4. **rrlab option flattening.** A nested list inside a list option prints as its Python repr. Proposal: give each
   inner list its own `option_item(name/i, j, v)` rows, or document the repr.

5. **names.rr `look`** climbs a block and its function separately. In `function f(A) { g(A) }` or
   `function f() { let a; a = 1; }`, a reference resolves to the local and to the module-level name both, because
   the function path skips the block and the block path skips the parameters. `ref_site` should give one site,
   the nearest. consistent-type-imports (`shadowed`) and explicit-module-boundary-types (`local_too`) work around
   it.

6. **rrlab / lowering.** A long `cat` message containing `'` and `. ` failed with "statement without a final
   '.'". Splitting it into smaller `cat`s works around it.

7. **eslint-a-functions has `fn_head`**, which follows ESLint core's getFunctionHeadLoc. typescript-eslint's
   version differs in two ways: a decorated class member starts after its decorators, and a function in a
   PropertyDefinition starts at the field. ts-a-returns therefore has its own `return_head`.

## Duplicates to merge

- ts-a-returns `return_head` and eslint-a-functions `fn_head`: one relation with a typescript-eslint variant.
- ts-b (ts-2 writer) and ts-1 share no names; check_rr is clean for both. Some relations are close in meaning
  and could be merged:
  - ts-b's `ts_module_decl` / `in_declare` and method-signature-style's module test (`no_fix`);
  - ts-b's `heritage_ref` and no-invalid-void-type's local `heritage`;
  - ts-b's `enum_member` / `enum_init` and no-duplicate-enum-values' local `enum_member_init`.
- Done after review: hasOverloadSignatures is ts-a `has_overloads` (explicit-module-boundary-types and
  no-invalid-void-type).
  - `last_decorator`, `class_method`, `object_method`, `property_def` and `heritage` are in ts-a too.
  - `const_body` builds on ts-a `const_cast`.
  - `held_name` (the declarator's name) is shared by the two return-type rules.
- explicit-member-accessibility's and member-ordering's decorator-aware member head (`head`, `report_at`) are
  close to ts-a-returns' `return_head`.
