# typescript-03: five typescript-eslint rules against their originals

Rules: `no-unnecessary-parameter-property-assignment`, `no-unused-private-class-members`, `no-unused-vars`,
`no-use-before-define`, `parameter-properties` (each `typescript-eslint-<name>`).

Ground truth: ESLint 9.39 with typescript-eslint 8.74.0 (`@typescript-eslint/parser` on every file, the rule at
"error" with its default options), run on a fleet box over the suite's `fixtures` (2,397 files) and
`rerail-typescript` (602 files) corpora and on the hand-written cases below, compared by (path, line) with
`--no-inline-config` (for no-unused-vars on rerail-typescript also with inline config: the same 80 keys). The
translations now report what ESLint reports on both corpora, line for line. Each changed translation's revision is one
up; parameter-properties changed only after the review (below).

## Results

| Rule | Status | What is left |
|---|---|---|
| no-unnecessary-parameter-property-assignment | same | |
| no-unused-private-class-members | same | |
| no-unused-vars | different | 6 translation-only keys, all ESLint's; the original is wrong |
| no-use-before-define | same | |
| parameter-properties | different | 2 original-only keys, ESLint has neither; the original is wrong |

## The shared cause: `earlier` has no rows on TypeScript

Three rules ordered nodes by core's `earlier` (`compound_before`, `later_member`, `later_param_used`). The engine
fills `visit_index`, which `earlier` lowers to, for Python only (`lib/tsx/next-01.rerail` says so), so on a .ts file
every `earlier` was false. Each now says `src_before` (next-01), start-offset order, which is ESLint's visit order for
the nodes compared here (assignments in one constructor, members of one class body, parameters of one function:
none nests in another).

## no-unnecessary-parameter-property-assignment: same

Fixed:
- `this.a += 1; this.a = a;` reported the second (013, 014): `earlier` had no rows.
- `a = a.trim(); this.a = a;` was reported (022–025, 044:8). typescript-eslint 8.74 skips a `this.x = x` after a
  write to the parameter x itself (`assignedBeforeUnnecessary`: an AssignmentExpression whose left is an identifier
  resolving to a parameter, in the constructor or an arrow IIFE there). New `param_write_before`.

Ad hoc, matching ESLint line for line: a write in an arrow IIFE before (`(() => { a = "x"; })(); this.a = a`) is not
reported; a write after, a write in a stored arrow (`const f = () => { a = "z"; }`), `a++` (an UpdateExpression, not
an assignment) all are; `this.a ||= a; this.a += "x"; this.a = a` reports the first only.

## no-unused-private-class-members: same

Fixed:
- A getter/setter pair, overloads and two constructors (adjacent-overload-signatures 031/067, explicit-module-boundary-
  types 007, parameter-properties 017/018, unified-signatures 049): the analyser's Map keeps the last member under a
  key, which `later_member` found by `earlier`.
- `const foo = ({ privateMember } = this) => {}` (no-unused-private-class-members 048): a parameter's default
  destructuring `this` reads the member, as an AssignmentPattern does; the pattern was looked for only on
  declarators and assignments.

## no-unused-vars: different, the original is wrong

Fixed:
- 320 translation-only keys on rerail-typescript and most fixture ones: `later_param_used` used `earlier`, so
  `args: "after-used"` exempted nothing (`function f(a, b) { return b; }` reported a).
- `using r = open();` and `await using s = open();` (no-unused-vars 234, 235) were never reported: the grammar now
  parses them as `using_declaration`, which `ts-b-scope`'s `variable` does not read. `lib/tsx/typescript-03.rerail`
  declares them.
- `{ [k in string]; }` (consistent-indexed-object-style 089): TypeScript parses it as a mapped type, whose key the
  rule marks used; tree-sitter recovers with the clause under an ERROR, not an index_signature, so the key had no
  "mappedType" scope. Any `mapped_type_clause` name is marked now.
- `export { C } from "./z"` read a local C (see the library below).
- `@Dec(M) class M {}`: the decorator reads the outer M (ESLint: used); tree-sitter hangs the decorator inside the
  class, where M resolves to the class's own name, a self reference. Found ad hoc.

Left, 6 translation-only keys, every one reported by ESLint:
- fixtures, 6: consistent-indexed-object-style 037:1, 038:1, 039:2, 040:2, 088:2, 089:2, each "'Foo' is defined but
  never used". `type Foo = { [key: string] };`, `{ [] }`, `interface Foo { [key: string]; }`: index signatures
  without a type, which TypeScript's parser accepts and tree-sitter does not. The original refuses these files
  ("source-incomplete: file has syntax errors") and reports nothing; ESLint, and the translation, report Foo.

The translation's `.scope` was `**/*`; it is the JSON's now (`.ts`, `.tsx`, `.mts`, `.cts`), which drops the one
rerail-typescript difference, `test/scripts/pack-test-data.test.mjs:1299` (a `.mjs` file the original never looks at;
ESLint reports `_licences` there only when configured to lint `.mjs` with the TypeScript parser).

## no-use-before-define: same

Fixed:
- `export type { A, … } from "./code-rule.js"` followed by `import type { A, … }` (rerail-typescript
  `src/engine/code-rule/code-rule-semantic.ts`, 9 keys): `ts-b-scope`'s `use` takes the names of a re-export's
  specifiers as value uses, and they resolved to the later import. The scope manager makes no reference for them.
  `reexport_name` (the library below) exempts them.
- `for (var a in a) {}`, `for (var a of a) {}` (065, 066, original-only): ESLint's isInInitializer requires the
  reference's scope to be the variable's; a `var` head makes no for scope in the scope manager, but `ts-b-scope`
  makes one for every for-in. `in_initialiser` steps over it for a `var` head.
- `@Directive({ useExisting: C }) export class C {}` (035:7): the class's own name in its decorator is exempt, but with
  `export` tree-sitter hangs the decorators on the export statement.

Ad hoc, matching ESLint: `x.go(); using x = open();` reported; `for (let b of b)`, `function f() { for (var e of e) {} }`
reported; `{ for (var c in c) {} }` not (the block is the reference's scope, not c's); `@Dec(N) export class N {}`
after `new N()` reports only the `new N()`.

## parameter-properties: different, the original is wrong

The translation reports what ESLint reports on both corpora (124 fixture keys, 25 rerail-typescript keys). The
original adds 2 on rerail-typescript, `src/engine/storage/database.ts:35` and `:267`:
`function connectSqlite(path: string, readonly: boolean)` and `function openChecked(path: string, readonly: boolean, …)`.
A parameter *named* `readonly` in a plain function is no parameter property; ESLint reports neither.

## Review fixes (review/typescript-03.review.md)

The reviewer's 18 defects, each confirmed against typescript-eslint 8.74.0 on a fleet box and fixed; each
counterexample is now a `.test` of its rule (`rerail-engine test-rr`: 15 tests, all pass), and the five rules' suite
verdicts are unchanged (ptcheck: three same; no-unused-vars and parameter-properties differ only as above).

no-unnecessary-parameter-property-assignment:
- An `accessor` field is an AccessorProperty, not a PropertyDefinition: a write in its value no longer counts as a
  field write (`accessor b = (this.a = "x")` before `this.a = a`: reported).
- A parenthesised field value `b = (foo(() => { this.a = "x"; }))` is the arrow IIFE's call (ESTree has no
  parentheses): a field write, so `this.a = a` is not reported.

no-unused-private-class-members:
- `accessor #x`: the analyser's PrivateIdentifier visitor skips the key of a method or PropertyDefinition only, so an
  AccessorProperty's own `#x` reads it; it is never reported.
- `const o: C = …; o.x`: getObjectClass returns null for a variable whose first definition is a declarator not
  `= this`; a variable's type is no longer read (a parameter's still is).
- `m(o: C<number>)`: a generic type reference names C (`ref_type_name`), for parameters and `this: C<T>` alike.
- `const { y: self } = this`: any name the declarator binds from `this` is self.
- `self++`, `[self] = …`, `for (self of …)`, `({ self } = …)`: any write reference other than the initialiser ends
  self (the analyser's `ref.isWrite() && ref.init !== true`).
- Keys: a literal by `String(value)` (`[true]`, `[null]`, `[1n]`), a template without substitutions by its raw
  text (`` [`a\x62`] `` is a\x62, so `this.ab` does not read it); member accesses `o[true]`, ``o[`k`]`` the same way.
- A class method's frame is the method whatever its `this` parameter says (`static n(this: B) { this.y }` reads the
  static map only); only other functions, an object literal's methods included, take `this: C`. A function that is
  an `accessor` field's value is no field-value frame either.

no-unused-vars:
- A parameter named only by a type predicate (`x is string`, `asserts y`) is "only used as a type"
  (`referenceContainsTypePredicate`). `ts-b-scope` makes no reference there (`in_predicate`); the rule adds the
  type-only read itself.
- `export interface Foo {}` with `const Foo = 1`: an interface or type alias of a variable that is also a value does
  not export it (`isMergedTypeDeclaration`): `merged_type`.
- `stored` is isStorableFunction now: from the reference's nearest function, climbing ESTree parents inside the right
  side, an argument (call, `new`), an assignment, a tagged template, a yield or a statement stores it; a callee or a
  non-last sequence element does not; anything else passes, and the right side's top stores nothing. So
  `a = [() => a]` and `e = { f: () => e }` are unused, `c = foo(() => c)` is used. Ad hoc, matching ESLint line for
  line: `(() => a)()`, `new Foo(() => b)`, a tagged template, a conditional, `as`, a spread, a sequence (either
  place), a function returning an arrow, an object method, `m += (() => m)()`.
- `args: "after-used"` counts a later parameter's references only when not type-only: `f(a, b): typeof b` reports
  both a and b.
- `ignoreRestSiblings`: a property with a default (`{ a = 1, ...rest }`) is no rest sibling (the name's parent is the
  AssignmentPattern, no Property). Ad hoc: `{ p: q = 1, ...rest }` reports q, `{ r: s, ...rest }` does not.

no-use-before-define:
- `typedefs: false` exempts type parameters too (`<T extends U, U>`): the scope manager defines them, as `infer U` and
  a mapped type's key, with a Type definition.

parameter-properties (revision 4):
- `abstract x` (TSAbstractPropertyDefinition) and `accessor x` (AccessorProperty) are not recorded fields.
- The constructor's run of assignments takes `this[x] = x` (a computed member whose property is an Identifier) and
  parenthesised targets `(this.q) = y`, `(this).q = y`.
- A computed key that is an identifier, `[x]: string`, is the field x (the rule reads `key.type` only).

Tests set no rule options (`option` is core's: a test cannot give it), so the option tests give the rule's own
relation instead: `.given prefer_params`, `.given opt`, `.given ignore_rest_siblings` (a relation no-unused-vars
now derives from the option for this).

## Library: `lib/tsx/typescript-03.rerail` (new)

Two corrections to `tsx/ts-b-scope`, shared by no-unused-vars and no-use-before-define. Both belong in ts-b-scope;
this file goes when it has them:

    variable(s, x, id, "using") :- kind(v, "using_declaration"), child(v, d, _), declarator_name(d, id, x),
        nearest(v, s, scope_node).
    // use(r, x, "value") should also require !reexport_name(r):
    reexport_name(r) :- kind(sp, "export_specifier"), (field(sp, "name", r) ; field(sp, "alias", r)),
        kind(r, "identifier"), nearest(sp, e, "export_statement"), field(e, "source", _).

Further `ts-b-scope` changes this batch worked around in its rules, for whoever next edits it:
- `for (var …)` heads make no scope (scope manager: only `let`/`const`/`using` heads nest a for scope).
- A decorator of a class is in the class's outer scope (the class name it reads is the outer variable), and with
  `export` it is the export statement's child.
- A type predicate's name (`x is T`, `asserts x`) is a type-only reference to the parameter, not no reference.

## Engine gaps

- `earlier`/`visit_index` have no rows for JavaScript and TypeScript under run-policies (as react-04 and next-01
  found). Rules here use next-01's `src_before`.
- The originals skip files tree-sitter cannot parse wholly (source-incomplete), where ESLint, whose parser accepts more,
  reports; that is the whole of no-unused-vars' fixture difference.
