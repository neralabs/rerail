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

No new library file: no concept is shared by two of these rules. One library fix, from the review: ts-a's
`member_key_name` decides quoting as TypeScript's requiresQuoting does (below).

## Review fixes (review/typescript-01.review.md)

Each counterexample of the review is a `.test` of its rule now, with the extra cases below checked against
typescript-eslint 8.74.0 on pt-w01; every test passes under the fleet's `rerail-engine test-rr`. A test cannot set
options (`test-rr` passes the run tool calls only), so a rule reads its options through one relation a test gives
with `.given`: `style` in class-literal-property-style, `setting` and `ignored_name` in
explicit-member-accessibility. ptcheck after the fixes: the same five statuses and counts as before, and
adjacent-overload-signatures and member-ordering (ts-a's other `ts_member_name` users) still `same`.

| rule | defect | fix |
|---|---|---|
| class-literal-property-style | 1: `get [k]` paired with `set [k]` by text | removed; a key pairs by getStaticMemberAccessValue only (`member_name`), and `[k]` of a parameter has none: reported, as 8.74 does. The "8.70 does" in the comment was wrong |
| | 2: constants not resolved | `key_value`: getStaticValue for computed keys and `this[k]`: literals, plain templates, `Symbol.x`, and a variable with one declarator definition that is `const` or never written after its initialiser (ts-b-scope's `resolves`/`writes`). `let k = "a"; k = "b"` is no constant (reported, as ESLint does). Not followed: operators, templates with `${…}`, other globals' members |
| | 3: constructor by its written name | `ts_ctor` (not static, key `constructor` or `"constructor"`) |
| | 4: exclusion lands on the constructor's class | it lands on the innermost class body around the `this` |
| | 5: `({ a: this.a } = o)` counted | `pair_pattern` dropped from `assignee`; `[this.a] = o` still counts |
| | 6: `readonly accessor e = 1` reported | tree-sitter misreads it (name `accessor`, `e` in an ERROR; `op` gives `readonly` only); a field holding an ERROR is left out |
| | also | the name "" pairs and excludes nothing (the plugin tests it for truth); `get [Symbol.iterator]` pairs with its `set` |
| class-methods-use-this | decorated field reported at its decorator | `head`: typescript-eslint's getFunctionHeadLoc, ts-a's `member_head` for a decorated field |
| consistent-indexed-object-style | 1: names by text | the owner (superVar) is the alias's or interface's variable; a name is followed as findVariable finds it from the owner's scope outwards (ts-b-scope), and the owner's name counts only while the owner has a reference (isNodeEqual compares identifiers by name: `type Bar<Foo> = Foo` is circular for Foo exactly when Foo is referenced somewhere) |
| | 2: type parameters of function types | the climb stops at TSTypeAnnotation only: annotations, and a function or constructor type's return type, which tree-sitter does not wrap |
| | 3: shadowed mapped key | a use of the key is a reference that resolves to the key's variable |
| explicit-member-accessibility | `"ä"` quoted | ts-a's `member_key_name`: `[$_\p{ID_Start}]` then `[$\p{ID_Continue}\u200C\u200D]*`, BMP only (TypeScript reads UTF-16 units: `"𝑎"` is quoted, `"a\u200c"` is not) |

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

## consistent-indexed-object-style: names resolve as the scope manager does; one key the original cannot see

The original reported `interface Foo<T> { [k: string]: T }` (065.ts:2), Foo1 in 072.ts and 073.ts, and Foo2 and Foo3
in 074.ts (lines 6 and 10), and typescript-eslint does too; the translation found them circular. Its `type_decl`
looked a type name up across the whole corpus, so `T` reached another fixture's `type T = …` and Foo2 another file's
Foo2. Names are now looked up in scopes (Review fixes, above), which keeps them in their file.

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

2. **`.expect r(…)` of a rule's own relation finds no rows.** In explicit-member-accessibility's test
   `the_names_of_string_keys`, `.expect missing(_, "method definition", "ä")` failed with "missing has 0 rows"
   (also with `.output missing` in the test) while the test's findings, which only `missing` derives, were there.
   A relation the test declares, `named(x) :- missing(_, _, x).`, has the rows; the test expects that instead.
