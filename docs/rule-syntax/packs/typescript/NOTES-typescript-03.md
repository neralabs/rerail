# typescript-03: five typescript-eslint rules against their originals

Rules: `no-unnecessary-parameter-property-assignment`, `no-unused-private-class-members`, `no-unused-vars`,
`no-use-before-define`, `parameter-properties` (each `typescript-eslint-<name>`).

Ground truth: ESLint 9.39 with typescript-eslint 8.74.0 (`@typescript-eslint/parser` on every file, the rule at
"error" with its default options), run on a fleet box over the suite's `fixtures` (2,397 files) and
`rerail-typescript` (602 files) corpora and on the hand-written cases below, compared by (path, line) with
`--no-inline-config` (for no-unused-vars on rerail-typescript also with inline config: the same 80 keys). The
translations now report what ESLint reports on both corpora, line for line. Each translation's revision is one up.

## Results

| Rule | Status | What is left |
|---|---|---|
| no-unnecessary-parameter-property-assignment | same | |
| no-unused-private-class-members | same | |
| no-unused-vars | different | 7 translation-only keys, all ESLint's; the original is wrong |
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

Left, 7 translation-only keys, every one reported by ESLint:
- fixtures, 6: consistent-indexed-object-style 037:1, 038:1, 039:2, 040:2, 088:2, 089:2, each "'Foo' is defined but
  never used". `type Foo = { [key: string] };`, `{ [] }`, `interface Foo { [key: string]; }`: index signatures
  without a type, which TypeScript's parser accepts and tree-sitter does not. The original refuses these files
  ("source-incomplete: file has syntax errors") and reports nothing; ESLint, and the translation, report Foo.
- rerail-typescript, 1: `test/scripts/pack-test-data.test.mjs:1299`,
  `const { "packs/p/…/LICENSES.md": _licences, ...withoutLicences } = …`. ESLint: "'_licences' is assigned a value
  but never used." (ignoreRestSiblings is false by default and the JSON sets no varsIgnorePattern.) The original
  misses a binding under a string-keyed pair pattern.

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

## Engine gaps

- `earlier`/`visit_index` have no rows for JavaScript and TypeScript under run-policies (as react-04 and next-01
  found). Rules here use next-01's `src_before`.
- The originals skip files tree-sitter cannot parse wholly (source-incomplete), where ESLint, whose parser accepts more,
  reports; that is the whole of no-unused-vars' fixture difference.
