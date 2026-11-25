# typescript-01: five typescript-eslint rules checked against their originals

Rules: class-literal-property-style, class-methods-use-this, consistent-indexed-object-style,
consistent-type-definitions, explicit-member-accessibility. Each was run alone with `translations:check` (ptcheck),
and every differing key was run through typescript-eslint 8.74.0 (ESLint 9.39.5, default options) on a fleet box.
The translation's own run on the same files was `rerail-engine run-policies` with this checkout's library.

| rule | status | right | what differs |
|---|---|---|---|
| class-literal-property-style | same | | was: an object literal's getter |
| class-methods-use-this | same | | was: an `accessor` field's arrow |
| consistent-indexed-object-style | different | translation | fixtures: 1 key in a file the original skips |
| consistent-type-definitions | different | translation | fixtures: 2 keys in files the original skips |
| explicit-member-accessibility | same | | was: engine gap 1, every member reported |

No new library file: no concept is shared by two of these rules.

## class-literal-property-style: a getter of a class only

The translation reported `get skip() { return false; }` in an object literal
(eslint-plugin-vitest/no-focused-tests/rerail-mode-option-overridden.test.ts:4); typescript-eslint does not: the
rule visits MethodDefinition, which an object's getter (a Property) is not. `literal_getter` now needs its getter
in a class body.

    class A { get a() { return 1; } }        reported
    const o = { get b() { return 1; } };     not reported

## class-methods-use-this: the finding's location is a loc

The original reported `accessor bar = (): void => { return; };` (explicit-module-boundary-types/028.ts:3, 070.ts:3)
and so does typescript-eslint ("Expected 'this' to be used by class arrow function."). The translation derived it,
but its violation took `at: node`, and the place it reports an `accessor` field's arrow is the `=>` token
(eslint-a-functions' `fn_head`, a `loc`, since an AccessorProperty is no holder), so the finding was dropped. The
violation now takes `at: loc`, as `fn_head` declares.

## consistent-indexed-object-style: names resolve in their own file; one key the original cannot see

The original reported `interface Foo<T> { [k: string]: T }` (065.ts:2), Foo1 in 072.ts and 073.ts, and Foo2 and Foo3
in 074.ts (lines 6 and 10), and typescript-eslint does too; the translation found them circular. Its `type_decl`
looked a type name up across the whole corpus, so `T` reached another fixture's `type T = …` and Foo2 another file's
Foo2. `named_decl` now needs the declaration in the name's own file (`docof`).

Left: 088.ts:2, `interface Foo { [key: string]: Bar; }` beside `interface Bar { [key: string]; }`. typescript-eslint
reports Foo (2:1). Bar's signature without a type is an error to tree-sitter, so the original skips the file; the
translation reports Foo. Translation right.

## consistent-type-definitions: a mapped type with an error; two keys the original cannot see

The translation reported `type Foo = { [k in string]; };` (consistent-indexed-object-style/089.ts:2); typescript-eslint
does not, as TSESTree reads a mapped type. tree-sitter reads an object type holding an ERROR around the
`k in string` clause. `object_alias` now leaves out an object type with such an error (`broken_mapped`).

Left: 037.ts:1 `type Foo = { [key: string] };` and 038.ts:1 `type Foo = { [] };`. typescript-eslint reports both
(1:6, "Use an `interface` instead of a `type`."); tree-sitter gives each an ERROR, so the original skips the files.
Translation right.

Library change proposed: ts-a's `type_literal(b)` should leave out the broken mapped type itself, as it leaves out a
well-formed one; then the rule needs no `broken_mapped`.

## explicit-member-accessibility: worked around engine gap 1

The translation reported every member, `public` or not (229 fixture keys, 58 in rerail-typescript, e.g.
class-literal-property-style/012.ts:3 `public readonly myButton = …`, customer-profile.ts:230
`private readonly profilesRoot`). The rule was right; the engine drops a negation (gap 1). The negations now join
`holder`, a predicate of tape facts only: `bare(m)` (no accessibility modifier) and `kept(m)` (not in
ignoredMethodNames), and `missing` and `unwanted` join them positively. On `class A { public a = 1; private n() {}
constructor(private readonly h: string) {} }` it reports only the constructor, as typescript-eslint does. With
ignoredMethodNames ["m"] (the option replaced by a fact in a probe copy; the suite runs defaults only),
`m() {}` is skipped and `m = 1` and `n() {}` reported, as typescript-eslint does.

## Engine gaps

1. **A negation after a join with a derived predicate that reads `value` is lost.** Seen with the fleet's
   `rerail-engine` (pt-w01, `~/pt/bin`) and the suite's. With `#include "tsx/eslint-a-common.rerail"` (or ts-a):

       .decl modified(m: node)
       modified(m) :- child(m, a, _), kind(a, "accessibility_modifier").
       .decl kc(k: node, x: symbol)
       kc(k, x) :- kind(k, "property_identifier"), text(k, x).
       kc(k, v) :- kind(k, "string"), value(k, v).
       .decl mc(m: node)
       mc(m) :- kind(m, "public_field_definition"), field(m, "name", k), kc(k, _).
       probe(m) :- mc(m), !modified(m).

   On `class B { public a = 1; }`, `probe` holds for `a`, though `modified(a)` holds (a positive `mc(m),
   modified(m)` finds it). It does not hold when `kc`'s second clause uses `text` instead of `value`, when `mc`'s
   body is inlined into `probe`, when the include is left out, or when the left side is a predicate of tape facts
   only. ts-a's `ts_member_name` is such a predicate, so any rule doing `ts_member_name`-derived `, !p(m)` may
   over-report. No `.decl` needed: an engine fix.
