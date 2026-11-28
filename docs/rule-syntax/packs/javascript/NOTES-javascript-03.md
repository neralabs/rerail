# Notes: batch javascript-03 (no-useless-call, prefer-exponentiation-operator, prefer-object-has-own, prefer-spread, sort-imports, sort-keys)

Six existing translations, brought to their JSON originals with `translations:check` (ptcheck on a fleet box), every
difference decided with ESLint 9.39 (`npx eslint@9`, flat config defaults: ecmaVersion latest, sourceType module;
typescript-eslint's parser for the `.ts` corpus). One new library, `lib/tsx/javascript-03.rerail` (`same_this`, for
no-useless-call and prefer-spread). Results: `javascript-03.result.json`.

## eslint-no-useless-call, eslint-prefer-spread

Followed: ESLint (and the JSON, which agrees). The receiver and the `this` argument are compared as ESLint's
equalTokens does. eslint-b-objects' `natural_this` compares `token_text`, which reads a node with no stored children
as its source text, spacing kept: `[]` and `[ ]` differed, and `[⏎/*empty*/⏎]` gave `[ ]`.

    [].concat.apply([ ], [1, 2]);      ESLint: no-useless-call        was missed (fixtures no-useless-call/032, 033)
    [].concat.apply([ ], args);        ESLint: prefer-spread          was missed (fixtures prefer-spread/022, 023)
    ({}).f.call({ }, 1);               ESLint: no-useless-call        was missed

Fix: `same_this` (new library) adds empty array and object literals, compared by kind, beside `natural_this`.

Engine gap (no-useless-call, fixtures `no-useless-call/034.js`): `abc.get("foo", 0).concat.apply(abc . get("foo",
0 ), [1, 2])` is reported when no-useless-call runs alone, and missed in a `run-policies` of no-useless-call and
eslint-prefer-spread only. The first two 6-rule ptchecks of this batch missed it too; the last one (all six rules
as committed) reports it, so the gap depends on the policy set run together. In the failing run `token_text` of the
receiver is `abc abc.get("foo", 0) . get ( " foo " , 0 )`:
the leaf clause of eslint-a-common's `tt_piece` (`token_text_ask(n), !token(n, …), text(n, x)`) fires for a node
that has stored tokens, so its whole text joins the pieces. Repro: `run-policies <dir with that line> <out>
zz.rerail eslint-prefer-spread.rerail`, zz printing `token_text` of `receiver(f, o)` and of the `this` argument; alone
the two texts are equal (`abc . get ( " foo " , 0 )`). Not fixable in the rule; an engine fix (a negation evaluated
against an incomplete relation when two policies share a library's demanded relation) is needed.

## eslint-prefer-exponentiation-operator

Followed: ESLint (and the JSON). ReferenceTracker reads a computed member's name with getStaticValue, which folds
constant strings; `global_member` reads only static_prop (a plain string or a template without `${}`).

    Math[`${'pow'}`](a, b)     Math['p' + 'o' + 'w'](a, b)     Math[true ? 'pow' : 'x'](a, b)    ESLint reports all
    const p = 'pow'; Math[p](a, b)                                                                ESLint: not reported

Fix: `math_pow` in the rule, tsx/values' `const_str` on the index (fixtures 034, 035). Library request: let
eslint-b-objects' `global_member` (and `global_named`) read computed names with `const_str`, so
`globalThis['Ma' + 'th'].pow` is followed too; the rule covers only the last member.

## eslint-prefer-object-has-own

Followed: ESLint (and the JSON). The rule's `global_at(c, "Object")` held where Object is a parameter, a module's
`var`, or turned off:

    (Object) => Object.hasOwnProperty.call(obj, prop)            fixtures 019, 039, 056
    /* global Object: off */ ({}).hasOwnProperty.call(a, b);     fixtures 059
    var Object = foo; Object.prototype.hasOwnProperty.call(o, p) fixtures 060 (a module: the var is not the global)

ESLint reports none of them. Engine gap: in `prefer_object_has_own(c) :- has_own_call(c, _), global_at(c,
"Object")`, `global_at`'s negations (`!declared_around`, `!named_expr_around`, `!global_off`) are dropped: a probe in
the same program reports `declared_around(c, "Object")` and `global_at(c, "Object")` for the same c. `global_at`
asked for every call (`kind(n, "call_expression")`) is right. Fix in the rule: `shadowed(c)`, the three tests written
positively and negated once.

## eslint-sort-imports

Followed: ESLint (and the JSON). Two gaps under tsx/modules:

1. `import_prev` has no rows: it orders by `visit_index`, and the engine gives no `visit_index` (nor `earlier`) for
   TypeScript. Every declaration-order finding was missed (1,603 on rerail-typescript, 50 in fixtures):
   `import os from "os";⏎import fs from "fs";` (ESLint: line 2). The rule orders imports by start offset (`before`),
   which is traversal order for import declarations. Engine or library request: fill `visit_index`/`earlier` for
   TypeScript, or have modules' `import_prev` use start offsets.
2. `import { default as foo, a } from "m"` (fixtures no-unused-vars/393): names.rerail's `import_binding` needs a
   `name` field, and tree-sitter keeps `default` as no node (only `alias` foo), so `import_member` misses foo, and the
   import reads as `single` with first name `a`. ESLint: "Member 'a' … should be sorted". The rule reads the
   specifiers itself (`member`, `member_at`, its own `first_local` and `import_syntax`). Library request: an
   `import_binding(sp, "foo", m, "default")` row for a specifier with an alias and no name.

Known limit, as before: names compare by code point, ESLint by UTF-16 unit (they differ only for a name with a
character above U+FFFF against one in U+E000-U+FFFF).

## eslint-sort-keys

Followed: ESLint (and the JSON). Keys compare by UTF-16 code unit in JavaScript, by code point here:

    expect(stableJsonText({ "\uE000": "private-use", "😀": "emoji" }))      ESLint: '😀' should be before '\uE000'

(rerail-typescript test/engine/stable-json.test.ts:12). Fix: `units`, which puts U+D7FF before each character above
U+FFFF, so it sorts where its high surrogate does; natural-compare also compares units, so natural keys use it too.
Checked against ESLint on `{ "😀b": 1, "😀a": 2 }`, `{ z, "😀", a }` and the case above. Library request: core's
`@js_compare` (js-2's request 5) would replace it and serve sort-imports too.
